use std::sync::Arc;

use flume::{Receiver, Sender};
use md5::{Digest, Md5};
use tracing::error;

use crate::{
    api::tracks::TrackInfo,
    audio::processor::DecoderCommand,
    sources::{http::HttpTrack, plugin::PlayableTrack, qobuz::token::QobuzTokenTracker},
};

pub struct QobuzTrack {
    pub info: TrackInfo,
    pub album_name: Option<String>,
    pub album_url: Option<String>,
    pub artist_url: Option<String>,
    pub artist_artwork_url: Option<String>,
    pub token_tracker: Arc<QobuzTokenTracker>,
    pub client: reqwest::Client,
}

impl PlayableTrack for QobuzTrack {
    fn start_decoding(
        &self,
    ) -> (
        Receiver<crate::audio::buffer::PooledBuffer>,
        Sender<DecoderCommand>,
        flume::Receiver<String>,
        Option<Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded::<crate::audio::buffer::PooledBuffer>(4);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let info = self.info.clone();
        let token_tracker = self.token_tracker.clone();
        let client = self.client.clone();

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            handle.block_on(async move {
                let url = match resolve_media_url(&client, &token_tracker, &info.identifier).await {
                    Ok(Some(url)) => Some(url),
                    Ok(None) => None,
                    Err(e) => {
                        error!(
                            "Qobuz: Failed to resolve media URL for {}: {}",
                            info.identifier, e
                        );
                        None
                    }
                };

                if let Some(url) = url {
                    let http_track = HttpTrack {
                        url,
                        local_addr: None,
                        proxy: None,
                    };
                    let (inner_rx, inner_cmd_tx, inner_err_rx, _inner_opus_rx) =
                        http_track.start_decoding();

                    // Proxy commands
                    let cmd_tx_clone = inner_cmd_tx.clone();
                    std::thread::spawn(move || {
                        while let Ok(cmd) = cmd_rx.recv() {
                            let _ = cmd_tx_clone.send(cmd);
                        }
                    });

                    // Proxy errors
                    std::thread::spawn(move || {
                        if let Ok(err) = inner_err_rx.recv() {
                            let _ = err_tx.send(err);
                        }
                    });

                    // Proxy samples
                    while let Ok(sample) = inner_rx.recv() {
                        if tx.send(sample).is_err() {
                            break;
                        }
                    }
                } else {
                    let _ = err_tx.send("Failed to resolve Qobuz media URL".to_string());
                }
            });
        });

        (rx, cmd_tx, err_rx, None)
    }
}

async fn resolve_media_url(
    client: &reqwest::Client,
    token_tracker: &QobuzTokenTracker,
    track_id: &str,
) -> crate::common::types::AnyResult<Option<String>> {
    let tokens = token_tracker
        .get_tokens()
        .await
        .ok_or_else(|| "Failed to get Qobuz tokens".to_string())?;

    if tokens.user_token.is_none() {
        return Ok(None);
    }

    let unix_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let format_id = "5";
    let intent = "stream";

    let sig_data = format!(
        "trackgetFileUrlformat_id{}intent{}track_id{}{}{}",
        format_id, intent, track_id, unix_ts, tokens.app_secret
    );
    let mut hasher = Md5::new();
    hasher.update(sig_data.as_bytes());
    let sig = hex::encode(hasher.finalize());

    let mut url = reqwest::Url::parse("https://www.qobuz.com/api.json/0.2/track/getFileUrl")?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("request_ts", &unix_ts.to_string());
        query.append_pair("request_sig", &sig);
        query.append_pair("track_id", track_id);
        query.append_pair("format_id", format_id);
        query.append_pair("intent", intent);
    }

    let mut request = client
        .get(url)
        .header("Accept", "application/json")
        .header("x-app-id", &tokens.app_id);

    if let Some(user_token) = &tokens.user_token {
        request = request.header("x-user-auth-token", user_token);
    }

    let resp = request.send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }

    let json: serde_json::Value = resp.json().await?;
    if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
        if let Some(is_sample) = json.get("sample").and_then(|v| v.as_bool()).or_else(|| {
            json.get("sample")
                .and_then(|v| v.as_str())
                .map(|s| s == "true")
        }) {
            if is_sample {
                return Ok(None);
            }
        }
        return Ok(Some(url.to_string()));
    }

    Ok(None)
}
