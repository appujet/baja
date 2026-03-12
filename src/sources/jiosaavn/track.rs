use std::net::IpAddr;

use base64::prelude::*;
use des::{
    Des,
    cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray},
};

use crate::{
    audio::{
        AudioFrame,
        processor::{AudioProcessor, DecoderCommand},
    },
    config::HttpProxyConfig,
    sources::plugin::{DecoderOutput, PlayableTrack},
};

pub struct JioSaavnTrack {
    pub encrypted_url: String,
    pub secret_key: Vec<u8>,
    pub is_320: bool,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for JioSaavnTrack {
    /// Starts decoding the track and returns channels for audio frames, decoder commands, and initialization errors.
    ///
    /// The method attempts to decrypt the track's encrypted URL, adjusts the playback URL to a 320kbps variant when `is_320` is true, spawns a blocking task that opens the stream and initializes the audio processor, and returns channel endpoints for consumption.
    ///
    /// # Returns
    ///
    /// A tuple `(rx, cmd_tx, err_rx)` where:
    /// - `rx` is a receiver of decoded `AudioFrame`s.
    /// - `cmd_tx` is a sender for `DecoderCommand`s to control the decoder.
    /// - `err_rx` is a receiver for up to one initialization error message (a `String`) produced if decryption, stream opening, or processor initialization fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use crate::tracks::JioSaavnTrack;
    /// # use crate::config::player::PlayerConfig;
    /// // Assuming `track` and `config` are available:
    /// // let (rx, cmd_tx, err_rx) = track.start_decoding(config);
    /// ```
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let mut playback_url = match self.decrypt_url(&self.encrypted_url) {
            Some(url) => url,
            None => {
                let (_tx, rx) = flume::bounded::<AudioFrame>(1);
                let (cmd_tx, _cmd_rx) = flume::unbounded::<DecoderCommand>();
                let (err_tx, err_rx) = flume::bounded::<String>(1);

                let _ = err_tx.send(
                    "Failed to decrypt JioSaavn URL. Check your secretKey in config.toml"
                        .to_owned(),
                );
                return (rx, cmd_tx, err_rx);
            }
        };

        if self.is_320 {
            playback_url = playback_url.replace("_96.mp4", "_320.mp4");
        }

        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let url = playback_url.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        tokio::task::spawn_blocking(move || {
            let reader = match super::reader::JioSaavnReader::new(&url, local_addr, proxy) {
                Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
                Err(e) => {
                    tracing::error!("Failed to create JioSaavnReader for {url}: {e}");
                    let _ = err_tx.send(format!("Failed to open stream: {e}"));
                    return;
                }
            };

            let kind = std::path::Path::new(&url)
                .extension()
                .and_then(|s| s.to_str())
                .map(crate::common::types::AudioFormat::from_ext);

            match AudioProcessor::new(reader, kind, tx, cmd_rx, Some(err_tx.clone()), config) {
                Ok(mut processor) => {
                    std::thread::Builder::new()
                        .name(format!("jiosaavn-decoder-{}", url))
                        .spawn(move || {
                            if let Err(e) = processor.run() {
                                tracing::error!(
                                    "JioSaavn audio processor error for {}: {}",
                                    url,
                                    e
                                );
                            }
                        })
                        .expect("failed to spawn jiosaavn decoder thread");
                }
                Err(e) => {
                    tracing::error!("JioSaavn failed to initialize processor for {}: {}", url, e);
                    let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                }
            }
        });

        (rx, cmd_tx, err_rx)
    }
}

impl JioSaavnTrack {
    fn decrypt_url(&self, encrypted: &str) -> Option<String> {
        if self.secret_key.len() != 8 {
            return None;
        }

        let cipher = Des::new_from_slice(&self.secret_key).ok()?;
        let mut data = BASE64_STANDARD.decode(encrypted).ok()?;

        for chunk in data.chunks_mut(8) {
            if chunk.len() == 8 {
                cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
            }
        }

        if let Some(&last_byte) = data.last() {
            let padding = last_byte as usize;
            if (1..=8).contains(&padding) && data.len() >= padding {
                data.truncate(data.len() - padding);
            }
        }

        String::from_utf8(data).ok()
    }
}
