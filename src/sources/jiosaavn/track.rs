use std::net::IpAddr;

use base64::prelude::*;
use des::{
    Des,
    cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray},
};
use tracing::{error, warn};

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
            let mut kind = std::path::Path::new(&url)
                .extension()
                .and_then(|s| s.to_str())
                .map(crate::common::types::AudioFormat::from_ext)
                .filter(|f| *f != crate::common::types::AudioFormat::Unknown)
                .or(Some(crate::common::types::AudioFormat::Mp4));

            let mut attempt = 0;
            const MAX_ATTEMPTS: u32 = 2;

            loop {
                attempt += 1;
                
                let reader = match super::reader::JioSaavnReader::new(&url, local_addr, proxy.clone()) {
                    Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
                    Err(e) => {
                        error!("Failed to create JioSaavnReader for {url}: {e}");
                        let _ = err_tx.send(format!("Failed to open stream: {e}"));
                        return;
                    }
                };

                match AudioProcessor::new(reader, kind, tx.clone(), cmd_rx.clone(), Some(err_tx.clone()), config.clone()) {
                    Ok(mut processor) => {
                        let thread_url = url.clone();
                        if let Err(e) = std::thread::Builder::new()
                            .name(format!("jiosaavn-decoder-{}", url))
                            .spawn(move || {
                                if let Err(e) = processor.run() {
                                    error!("JioSaavn audio processor error for {}: {}", thread_url, e);
                                }
                            }) 
                        {
                            error!("JioSaavn: failed to spawn decoder thread: {e}");
                            let _ = err_tx.send(format!("Failed to spawn decoder thread: {e}"));
                        }
                        return;
                    }
                    Err(e) if attempt < MAX_ATTEMPTS => {
                        warn!("JioSaavn: processor init failed for {} (attempt {}): {} — retrying without format hint", url, attempt, e);
                        kind = None;
                        continue;
                    }
                    Err(e) => {
                        error!("JioSaavn failed to initialize processor for {}: {}", url, e);
                        let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                        return;
                    }
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
