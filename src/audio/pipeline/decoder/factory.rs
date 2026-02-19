use crate::audio::HlsReader;
use crate::audio::pipeline::decoder::context::DecoderContext;
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

/// Specialized factory to produce the right MediaSource for any given context.
pub fn create_reader(
    ctx: &DecoderContext,
) -> Result<Box<dyn MediaSource>, Box<dyn std::error::Error + Send + Sync>> {
    let url = &ctx.url;

    // 1. YouTube SABR
    if url.starts_with("sabr://") {
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
        return Ok(Box::new(reader));
    }

    // 2. HLS Manifests
    if is_hls_url(url) {
        debug!("Creating HLS reader for: {}", url);
        let player_url = if url.contains("youtube.com") {
            Some(url.to_string())
        } else {
            None
        };
        return HlsReader::new(url, ctx.local_addr, ctx.cipher_manager.clone(), player_url)
            .map(|r| Box::new(r) as Box<dyn MediaSource>);
    }

    // 3. Deezer Encrypted
    if url.starts_with("deezer_encrypted:") {
        let prefix_len = "deezer_encrypted:".len();
        if let Some(rest) = url.get(prefix_len..) {
            if let Some(colon_pos) = rest.find(':') {
                let track_id = &rest[..colon_pos];
                let real_url = &rest[colon_pos + 1..];

                let config = crate::configs::base::Config::load().unwrap_or_default();
                let master_key = config
                    .deezer
                    .as_ref()
                    .and_then(|c| c.master_decryption_key.clone())
                    .unwrap_or_default();

                return Ok(Box::new(crate::audio::DeezerReader::new(
                    real_url,
                    track_id,
                    &master_key,
                    ctx.local_addr,
                    ctx.proxy.clone(),
                )?));
            }
        }
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Malformed Deezer URL",
        )));
    }

    // 4. Default Remote Reader
    Ok(Box::new(crate::audio::reader::RemoteReader::new(
        url,
        ctx.local_addr,
        ctx.proxy.clone(),
    )?))
}

fn is_hls_url(url: &str) -> bool {
    url.contains(".m3u8") || url.contains("/api/manifest/hls_")
}
