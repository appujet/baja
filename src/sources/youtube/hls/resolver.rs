use std::sync::Arc;

use super::{
  parser::parse_m3u8,
  types::{M3u8Playlist, Resource},
};
use crate::{common::types::AnyResult, sources::youtube::cipher::YouTubeCipherManager};

pub async fn resolve_playlist(
  client: &reqwest::Client,
  url: &str,
) -> AnyResult<(Vec<Resource>, Option<Resource>)> {
  let text = fetch_text(client, url).await?;
  let playlist = parse_m3u8(&text, url);

  match playlist {
    M3u8Playlist::Master {
      variants,
      audio_groups,
    } => {
      let best = variants
        .iter()
        .filter(|v| v.is_audio_only)
        .max_by_key(|v| v.bandwidth)
        .or_else(|| {
          variants
            .iter()
            .filter(|v| v.audio_group.is_some())
            .max_by_key(|v| v.bandwidth)
        })
        .or_else(|| variants.iter().max_by_key(|v| v.bandwidth));

      match best {
        Some(v) => {
          // If the variant has an audio group, try to find a rendition URI.
          if let Some(group_id) = &v.audio_group {
            if let Some(group) = audio_groups.get(group_id) {
              let rendition = group
                .iter()
                .find(|m| m.is_default)
                .or_else(|| group.iter().find(|m| m.uri.is_some()))
                .and_then(|m| m.uri.as_ref());

              if let Some(uri) = rendition {
                tracing::debug!("HLS: selected audio group {} -> {}", group_id, uri);
                return Box::pin(resolve_playlist(client, uri)).await;
              }
            }
          }

          tracing::debug!(
            "HLS: selected variant bw={} codecs={:?} audio_only={} audio_group={:?} url={}",
            v.bandwidth,
            v.codecs,
            v.is_audio_only,
            v.audio_group,
            v.url
          );
          Box::pin(resolve_playlist(client, &v.url)).await
        }
        None => Err("HLS master playlist has no variants".into()),
      }
    }
    M3u8Playlist::Media { segments, map } => Ok((segments, map)),
  }
}

pub async fn fetch_text(client: &reqwest::Client, url: &str) -> AnyResult<String> {
  let res = client
    .get(url)
    .header("Accept", "application/x-mpegURL, */*")
    .send()
    .await?;

  if !res.status().is_success() {
    return Err(format!("HLS playlist fetch failed {}: {}", res.status(), url).into());
  }

  let text = res.text().await?;
  Ok(text)
}

pub fn resolve_url_string(
  url: &str,
  cipher_manager: &Option<Arc<YouTubeCipherManager>>,
  player_url: &Option<String>,
) -> AnyResult<String> {
  let (cipher, p_url) = match (cipher_manager, player_url) {
    (Some(c), Some(p)) => (c, p),
    _ => return Ok(url.to_string()),
  };

  let n_token = if let Some(pos) = url.find("/n/") {
    let rest = &url[pos + 3..];
    rest.split('/').next()
  } else {
    url
      .split("&n=")
      .nth(1)
      .or_else(|| url.split("?n=").nth(1))
      .and_then(|s| s.split('&').next())
  };

  if let Some(n) = n_token {
    let handle = tokio::runtime::Handle::current();
    let cipher = cipher.clone();
    let url_str = url.to_string();
    let p_url_str = p_url.clone();
    let n_str = n.to_string();

    Ok(handle.block_on(async move {
      cipher
        .resolve_url(&url_str, &p_url_str, Some(&n_str), None)
        .await
    })?)
  } else {
    Ok(url.to_string())
  }
}
