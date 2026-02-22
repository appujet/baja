use flume::{Receiver, Sender};

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    sources::plugin::PlayableTrack,
};

pub struct MixcloudTrack {
    pub hls_url: Option<String>,
    pub stream_url: Option<String>,
    pub uri: String,
    pub local_addr: Option<std::net::IpAddr>,
}

impl PlayableTrack for MixcloudTrack {
    fn start_decoding(
        &self,
    ) -> (
        Receiver<Vec<i16>>,
        Sender<DecoderCommand>,
        Receiver<String>,
    ) {
        let (tx, rx) = flume::bounded::<Vec<i16>>(64);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let hls_url = self.hls_url.clone();
        let stream_url = self.stream_url.clone();
        let local_addr = self.local_addr;


        tokio::spawn(async move {
            let (reader, kind) = if let Some(url) = hls_url {
                (
                    crate::sources::youtube::hls::HlsReader::new(&url, local_addr, None, None, None)
                        .ok()
                        .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>),
                    Some(crate::common::types::AudioKind::Aac) // HLS in mixcloud is usually ts containing AAC
                )
            } else if let Some(url) = stream_url {
                match super::reader::MixcloudReader::new(&url, local_addr) {
                    Ok(r) => (
                        Some(Box::new(r) as Box<dyn symphonia::core::io::MediaSource>),
                        std::path::Path::new(&url)
                            .extension()
                            .and_then(|s| s.to_str())
                            .and_then(crate::common::types::AudioKind::from_ext)
                            .or(Some(crate::common::types::AudioKind::Mp4))
                    ),
                    Err(_) => (None, None)
                }
            } else {
                (None, None)
            };

            if let Some(r) = reader {
                match AudioProcessor::new(r, kind, tx, cmd_rx, Some(err_tx)) {
                    Ok(mut processor) => {
                        if let Err(e) = processor.run() {
                            tracing::error!("Mixcloud audio processor error: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Mixcloud failed to initialize processor: {}", e);
                    }
                }
            } else {
                tracing::error!("Mixcloud: failed to create reader");
            }
        });

        (rx, cmd_tx, err_rx)
    }
}
