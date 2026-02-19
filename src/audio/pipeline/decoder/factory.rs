use crate::audio::pipeline::decoder::context::DecoderContext;
use crate::audio::{DeezerReader, HlsReader, RemoteReader};
use crate::sources::youtube::sabr::reader::SabrReader;
use crate::sources::youtube::sabr::structs::FormatId;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::Deserialize;
use symphonia::core::io::MediaSource;
use tracing::debug;

#[derive(Deserialize)]
struct SabrData {
    url: String,
    config: String,
    #[serde(rename = "clientName")]
    client_name: i32,
    #[serde(rename = "clientVersion")]
    client_version: String,
    #[serde(rename = "visitorData")]
    visitor_data: String,
    formats: Vec<FormatId>,
}

enum SourceType {
    Sabr,
    Hls,
    Deezer,
    Generic,
}

impl SourceType {
    fn from_url(url: &str) -> Self {
        if url.starts_with("sabr://") {
            SourceType::Sabr
        } else if url.contains(".m3u8") || url.contains("/api/manifest/hls_") {
            SourceType::Hls
        } else if url.starts_with("deezer_encrypted:") {
            SourceType::Deezer
        } else {
            SourceType::Generic
        }
    }
}

/// Specialized factory to produce the right MediaSource for any given context.
pub fn create_reader(
    ctx: &DecoderContext,
) -> Result<Box<dyn MediaSource>, Box<dyn std::error::Error + Send + Sync>> {
    let url = &ctx.url;
    let source_type = SourceType::from_url(url);

    match source_type {
        SourceType::Sabr => {
            let encoded = &url[7..];
            let decoded = BASE64_STANDARD.decode(encoded)?;
            let sabr_data: SabrData = serde_json::from_slice(&decoded)?;

            let config_bytes = if sabr_data.config.is_empty() {
                Vec::new()
            } else {
                BASE64_STANDARD.decode(sabr_data.config)?
            };

            let (reader, _) = SabrReader::new(
                sabr_data.url,
                config_bytes,
                sabr_data.client_name,
                sabr_data.client_version,
                sabr_data.visitor_data,
                sabr_data.formats,
            );
            Ok(Box::new(reader))
        }

        SourceType::Hls => {
            debug!("Creating HLS reader for: {}", url);
            let player_url = if url.contains("youtube.com") {
                Some(url.to_string())
            } else {
                None
            };
            HlsReader::new(url, ctx.local_addr, ctx.cipher_manager.clone(), player_url)
                .map(|r| Box::new(r) as Box<dyn MediaSource>)
        }

        SourceType::Deezer => {
            let rest = &url["deezer_encrypted:".len()..];
            let (track_id, real_url) = rest.split_once(':').ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Malformed Deezer URL")
            })?;

            let config = crate::configs::base::Config::load().unwrap_or_default();
            let master_key = config
                .deezer
                .as_ref()
                .and_then(|c| c.master_decryption_key.clone())
                .unwrap_or_default();

            Ok(Box::new(DeezerReader::new(
                real_url,
                track_id,
                &master_key,
                ctx.local_addr,
                ctx.proxy.clone(),
            )?))
        }

        SourceType::Generic => Ok(Box::new(RemoteReader::new(
            url,
            ctx.local_addr,
            ctx.proxy.clone(),
        )?)),
    }
}
