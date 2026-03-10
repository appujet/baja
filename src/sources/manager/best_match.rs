use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};

use crate::sources::{manager::SourceManager, plugin::BoxedTrack};

pub struct MirrorResult {
    pub track: BoxedTrack,
    pub score: f64,
    pub provider: String,
}

fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();

    let mut stripped = String::with_capacity(lower.len());
    let mut depth: usize = 0;
    for ch in lower.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => stripped.push(ch),
            _ => {}
        }
    }

    let stripped = stripped
        .replace("feat.", " ")
        .replace("feat ", " ")
        .replace("ft.", " ")
        .replace("ft ", " ");

    let clean: String = stripped
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect();

    clean.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

fn string_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let na = normalize(a);
    let nb = normalize(b);
    if na == nb {
        return 1.0;
    }
    if na.contains(&nb) || nb.contains(&na) {
        let shorter = na.len().min(nb.len()) as f64;
        let longer = na.len().max(nb.len()) as f64;
        return 0.80 + (shorter / longer) * 0.15;
    }
    let max_len = na.len().max(nb.len());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&na, &nb) as f64 / max_len as f64
}

fn duration_similarity(d1: u64, d2: u64, tolerance_ms: u64) -> f64 {
    if d1 == 0 || d2 == 0 {
        return 0.5;
    }
    let diff = d1.abs_diff(d2);
    if diff <= tolerance_ms {
        1.0
    } else {
        (1.0 - diff as f64 / d1.max(d2) as f64).max(0.0)
    }
}

fn score_match(
    orig_title: &str,
    orig_author: &str,
    orig_length: u64,
    cand_title: &str,
    cand_author: &str,
    cand_length: u64,
    cfg: &crate::config::server::BestMatchConfig,
) -> f64 {
    let nt = normalize(orig_title);
    let nc = normalize(cand_title);

    let title_score = if nt == nc {
        1.0
    } else if nc.starts_with(&nt) {
        0.95
    } else if nc.contains(&nt) || nt.contains(&nc) {
        let shorter = nt.len().min(nc.len()) as f64;
        let longer = nt.len().max(nc.len()) as f64;
        0.82 + (shorter / longer) * 0.10
    } else {
        string_similarity(&nt, &nc)
    };

    title_score * cfg.weight_title
        + string_similarity(orig_author, cand_author) * cfg.weight_artist
        + duration_similarity(orig_length, cand_length, cfg.duration_tolerance_ms)
            * cfg.weight_duration
}

fn fmt_ms(ms: u64) -> String {
    let s = ms / 1_000;
    format!("{}:{:02}", s / 60, s % 60)
}

pub async fn resolve_scored(
    manager: &SourceManager,
    track_info: &crate::protocol::tracks::TrackInfo,
    identifier: &str,
    mirrors: &crate::config::server::MirrorsConfig,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
) -> Option<BoxedTrack> {
    let isrc = track_info.isrc.as_deref().unwrap_or("");
    let query = format!("{} {}", track_info.title, track_info.author);
    let cfg = &mirrors.best_match;

    let original_source_name = manager
        .sources
        .iter()
        .find(|s| s.can_handle(identifier))
        .map(|s| s.name().to_string());

    let mut free_providers: Vec<String> = Vec::new();
    let mut throttled_providers: Vec<String> = Vec::new();

    for provider in &mirrors.providers {
        if isrc.is_empty() && provider.contains("%ISRC%") {
            tracing::debug!("Skipping mirror provider '{}': track has no ISRC", provider);
            continue;
        }

        let resolved = provider.replace("%ISRC%", isrc).replace("%QUERY%", &query);

        if let Some(src) = manager.sources.iter().find(|s| s.can_handle(&resolved)) {
            if src.is_mirror() {
                tracing::warn!(
                    "Skipping mirror provider '{}': '{}' is a Mirror-type source",
                    resolved,
                    src.name()
                );
                continue;
            }
            if Some(src.name().to_string()) == original_source_name {
                tracing::debug!(
                    "Skipping mirror provider '{}': would loop back to '{}'",
                    resolved,
                    src.name()
                );
                continue;
            }
        }

        if cfg
            .throttled_prefixes
            .iter()
            .any(|p| resolved.starts_with(p.as_str()))
        {
            throttled_providers.push(resolved);
        } else {
            free_providers.push(resolved);
        }
    }

    let mut global_best: Option<MirrorResult> = None;

    if !free_providers.is_empty() {
        let mut futs: FuturesUnordered<_> = free_providers
            .iter()
            .map(|p| search_provider(manager, track_info, p, routeplanner.clone(), cfg, false))
            .collect();

        while let Some(result) = futs.next().await {
            if let Some(mr) = result {
                tracing::info!(
                    "[Mirror] \"{}\" | {} | {} => {} | score: {:.3}",
                    track_info.title,
                    track_info.author,
                    fmt_ms(track_info.length),
                    mr.provider,
                    mr.score,
                );

                if mr.score >= cfg.immediate_use {
                    return Some(mr.track);
                }

                if global_best.as_ref().map_or(true, |b| mr.score > b.score) {
                    global_best = Some(mr);
                }
            }
        }

        if global_best
            .as_ref()
            .is_some_and(|b| b.score >= cfg.immediate_use)
        {
            let best = global_best.unwrap();
            tracing::info!(
                "[Mirror] free-phase winner \"{}\" via {} (score {:.3})",
                track_info.title,
                best.provider,
                best.score
            );
            return Some(best.track);
        }
    }

    for provider in &throttled_providers {
        if let Some(mr) =
            search_provider(manager, track_info, provider, routeplanner.clone(), cfg, true).await
        {
            tracing::info!(
                "[Mirror] throttled match \"{}\" via {} (score {:.3})",
                track_info.title,
                mr.provider,
                mr.score
            );
            return Some(mr.track);
        }
    }

    if let Some(best) = global_best {
        tracing::info!(
            "[Mirror] fallback match \"{}\" via {} (score {:.3})",
            track_info.title,
            best.provider,
            best.score
        );
        return Some(best.track);
    }

    tracing::warn!(
        "[Mirror] no valid mirror found for \"{}\" | {}",
        track_info.title,
        track_info.author
    );
    None
}

async fn search_provider(
    manager: &SourceManager,
    original: &crate::protocol::tracks::TrackInfo,
    resolved_provider: &str,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    cfg: &crate::config::server::BestMatchConfig,
    trust_any: bool,
) -> Option<MirrorResult> {
    use crate::protocol::tracks::LoadResult;

    let candidates: Vec<crate::protocol::tracks::TrackInfo> =
        match manager.load(resolved_provider, routeplanner.clone()).await {
            LoadResult::Track(t) => vec![t.info],
            LoadResult::Search(tracks) => tracks.into_iter().take(10).map(|t| t.info).collect(),
            _ => return None,
        };

    if candidates.is_empty() {
        return None;
    }

    let mut scored: Vec<(f64, crate::protocol::tracks::TrackInfo)> = candidates
        .into_iter()
        .map(|info| {
            let s = score_match(
                &original.title,
                &original.author,
                original.length,
                &info.title,
                &info.author,
                info.length,
                cfg,
            );
            (s, info)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let top_score = scored[0].0;

    let (limit, threshold): (usize, f64) = if trust_any {
        (scored.len(), 0.0)
    } else if top_score >= cfg.immediate_use {
        (1, cfg.immediate_use)
    } else if top_score >= cfg.high_confidence {
        (2, cfg.high_confidence)
    } else {
        (3, cfg.min_similarity)
    };

    for (score, info) in scored.into_iter().take(limit) {
        if score < threshold {
            break;
        }
        let id = info.uri.as_deref().unwrap_or(&info.identifier);
        if let Some(track) = super::resolver::resolve_nested_track(manager, id, routeplanner.clone()).await {
            return Some(MirrorResult {
                track,
                score,
                provider: resolved_provider.to_string(),
            });
        }
    }

    None
}