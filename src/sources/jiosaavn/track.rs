use std::net::IpAddr;

use base64::prelude::*;
use des::{
    Des,
    cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray},
};
use flume::{Receiver, Sender};

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    configs::HttpProxyConfig,
    sources::plugin::PlayableTrack,
};

pub struct JioSaavnTrack {
    pub encrypted_url: String,
    pub secret_key: Vec<u8>,
    pub is_320: bool,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for JioSaavnTrack {
    fn start_decoding(
        &self,
    ) -> (
        Receiver<Vec<i16>>,
        Sender<DecoderCommand>,
        flume::Receiver<String>,
    ) {
        let mut playback_url = self
            .decrypt_url(&self.encrypted_url)
            .expect("Failed to decrypt JioSaavn URL");

        if self.is_320 {
            playback_url = playback_url.replace("_96.mp4", "_320.mp4");
        }

        let (tx, rx) = flume::bounded::<Vec<i16>>(64);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let url = playback_url.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            let reader = match super::reader::JioSaavnReader::new(&url, local_addr, proxy) {
                Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
                Err(e) => {
                    tracing::error!("Failed to create JioSaavnReader for JioSaavn: {}", e);
                    let _ = err_tx.send(format!("Failed to open stream: {}", e));
                    return;
                }
            };

            let kind = std::path::Path::new(&url)
                .extension()
                .and_then(|s| s.to_str())
                .and_then(crate::common::types::AudioKind::from_ext);

            match AudioProcessor::new(reader, kind, tx, cmd_rx, Some(err_tx)) {
                Ok(mut processor) => {
                    if let Err(e) = processor.run() {
                        tracing::error!("JioSaavn audio processor error: {}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("JioSaavn failed to initialize processor: {}", e);
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

        if let Some(last_byte) = data.last() {
            let padding = *last_byte as usize;
            if padding > 0 && padding <= 8 {
                let len = data.len();
                if len >= padding {
                    data.truncate(len - padding);
                }
            }
        }

        String::from_utf8(data).ok()
    }
}
