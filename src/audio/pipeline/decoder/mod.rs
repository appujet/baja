pub mod context;
pub mod factory;
pub mod processor;

use self::context::DecoderContext;
use self::processor::AudioProcessor;
use crate::configs::HttpProxyConfig;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use flume::{Receiver, Sender};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum DecoderCommand {
    Seek(u64), // Position in milliseconds
    Stop,
}

/// The main entry point for the audio decoding pipeline.
pub fn start_decoding(
    url: String,
    local_addr: Option<std::net::IpAddr>,
    cipher_manager: Option<Arc<YouTubeCipherManager>>,
    proxy: Option<HttpProxyConfig>,
) -> (Receiver<i16>, Sender<DecoderCommand>) {
    let ctx = DecoderContext::new(url, local_addr, cipher_manager, proxy);

    let (tx, rx) = flume::bounded::<i16>(4096 * 4);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

    let ctx_clone = ctx.clone();
    std::thread::spawn(move || {
        let reader = match factory::create_reader(&ctx_clone) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to create reader for {}: {}", ctx_clone.url, e);
                return;
            }
        };

        match AudioProcessor::new(&ctx_clone.url, reader, tx, cmd_rx) {
            Ok(mut processor) => {
                if let Err(e) = processor.run() {
                    tracing::error!("Audio processor error: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to initialize audio processor: {}", e);
            }
        }
    });

    (rx, cmd_tx)
}
