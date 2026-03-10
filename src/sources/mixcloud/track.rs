use std::sync::Arc;

use crate::{
    audio::{
        AudioFrame,
        processor::{AudioProcessor, DecoderCommand},
    },
    sources::plugin::{DecoderOutput, PlayableTrack},
};

pub struct MixcloudTrack {
    pub client: Arc<reqwest::Client>,
    pub hls_url: Option<String>,
    pub stream_url: Option<String>,
    pub uri: String,
    pub local_addr: Option<std::net::IpAddr>,
}

impl PlayableTrack for MixcloudTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let uri = self.uri.clone();
        let client = self.client.clone();
        let hls_url_opt = self.hls_url.clone();
        let stream_url_opt = self.stream_url.clone();
        let local_addr = self.local_addr;

        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _guard = handle.enter();
            handle.block_on(async move {
                let (hls_url, stream_url) = if hls_url_opt.is_some() || stream_url_opt.is_some() {
                    (hls_url_opt, stream_url_opt)
                } else {
                    let (enc_hls, enc_url) = super::fetch_track_stream_info(&client, &uri)
                        .await
                        .unwrap_or((None, None));
                    (
                        enc_hls.map(|s| super::decrypt(&s)),
                        enc_url.map(|s| super::decrypt(&s)),
                    )
                };

                let (reader, kind) = if let Some(url) = hls_url {
                    match crate::sources::youtube::hls::HlsReader::new(
                        &url, local_addr, None, None, None,
                    ) {
                        Ok(r) => (
                            Some(Box::new(r) as Box<dyn symphonia::core::io::MediaSource>),
                            Some(crate::common::types::AudioFormat::Aac),
                        ),
                        Err(e) => {
                            tracing::error!("Mixcloud HlsReader failed to initialize: {e}");
                            (None, None)
                        }
                    }
                } else if let Some(url) = stream_url {
                    match super::reader::MixcloudReader::new(&url, local_addr) {
                        Ok(r) => (
                            Some(Box::new(r) as Box<dyn symphonia::core::io::MediaSource>),
                            std::path::Path::new(&url)
                                .extension()
                                .and_then(|s| s.to_str())
                                .map(crate::common::types::AudioFormat::from_ext)
                                .or(Some(crate::common::types::AudioFormat::Mp4)),
                        ),
                        Err(e) => {
                            tracing::error!("MixcloudReader failed to initialize: {e}");
                            (None, None)
                        }
                    }
                } else {
                    (None, None)
                };

                if let Some(r) = reader {
                    match AudioProcessor::new(r, kind, tx, cmd_rx, Some(err_tx.clone()), config) {
                        Ok(mut processor) => {
                            if let Err(e) = processor.run() {
                                tracing::error!("Mixcloud audio processor error: {e}");
                            }
                        }
                        Err(e) => {
                            tracing::error!("Mixcloud failed to initialize processor: {e}");
                            let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                        }
                    }
                } else {
                    tracing::error!("Mixcloud: failed to create reader");
                    let _ = err_tx.send("Mixcloud: failed to create reader".to_owned());
                }
            });
        });

        (rx, cmd_tx, err_rx)
    }
}
