use std::sync::Arc;

use crate::sources::{manager::SourceManager, plugin::BoxedTrack};

/// Fallback mechanism to resolve a track using mirrors (ISRC or search queries).
pub async fn resolve_with_mirrors(
    manager: &SourceManager,
    track_info: &crate::protocol::tracks::TrackInfo,
    identifier: &str,
    mirrors: &crate::config::server::MirrorsConfig,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
) -> Option<BoxedTrack> {
    if mirrors.best_match.scoring {
        return super::best_match::resolve_scored(manager, track_info, identifier, mirrors, routeplanner).await;
    }

    let isrc = track_info.isrc.as_deref().unwrap_or("");
    let query = format!("{} - {}", track_info.title, track_info.author);

    let original_source_name = manager
        .sources
        .iter()
        .find(|s| s.can_handle(identifier))
        .map(|s| s.name());

    for provider in &mirrors.providers {
        if isrc.is_empty() && provider.contains("%ISRC%") {
            tracing::debug!("Skipping mirror provider '{}': track has no ISRC", provider);
            continue;
        }

        let resolved = provider.replace("%ISRC%", isrc).replace("%QUERY%", &query);

        if let Some(handling_source) = manager.sources.iter().find(|s| s.can_handle(&resolved)) {
            if handling_source.is_mirror() {
                tracing::warn!(
                    "Skipping mirror provider '{}': '{}' is a Mirror-type source",
                    resolved,
                    handling_source.name()
                );
                continue;
            }
            if Some(handling_source.name()) == original_source_name {
                tracing::debug!(
                    "Skipping mirror provider '{}': would loop back to '{}'",
                    resolved,
                    handling_source.name()
                );
                continue;
            }
        }

        let res = match manager.load(&resolved, routeplanner.clone()).await {
            crate::protocol::tracks::LoadResult::Track(t) => {
                let id = t.info.uri.as_deref().unwrap_or(&t.info.identifier);
                resolve_nested_track(manager, id, routeplanner.clone()).await
            }
            crate::protocol::tracks::LoadResult::Search(tracks) => {
                if let Some(first) = tracks.first() {
                    let id = first.info.uri.as_deref().unwrap_or(&first.info.identifier);
                    resolve_nested_track(manager, id, routeplanner.clone()).await
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(track) = res {
            return Some(track);
        }
    }

    None
}

/// Helper to resolve a playable track from a source after a mirror redirect.
pub async fn resolve_nested_track(
    manager: &SourceManager,
    identifier: &str,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
) -> Option<BoxedTrack> {
    for source in &manager.sources {
        if source.can_handle(identifier)
            && let Some(track) = source.get_track(identifier, routeplanner.clone()).await
        {
            return Some(track);
        }
    }
    None
}