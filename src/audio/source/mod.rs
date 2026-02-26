//! `AudioSource` trait — the common contract for all readable audio sources.
//!
//! Both [`HttpSource`] and [`SegmentedSource`] implement this trait, as well
//! as any custom per-source reader a `PlayableTrack` may produce.
//!
//! # Module layout
//!
//! ```text
//! src/audio/source/
//! ├── mod.rs         ← AudioSource trait + create_client helper + re-exports
//! ├── http.rs        ← HttpSource   (prefetch thread, streaming HTTP)
//! └── segmented.rs   ← SegmentedSource (parallel chunk workers, seekable)
//! ```
//!
//! # Choosing a source
//!
//! | Use case                              | Source           |
//! |---------------------------------------|------------------|
//! | Generic stream (JioSaavn, Gaana, …)   | [`HttpSource`]   |
//! | Chunked / seekable (YouTube, HLS, …)  | [`SegmentedSource`] |

pub mod http;
pub mod segmented;

pub use http::HttpSource;
pub use segmented::SegmentedSource;

use std::io::{Read, Seek};

use symphonia::core::io::MediaSource;

// ─── AudioSource trait ────────────────────────────────────────────────────────

/// Common trait implemented by every readable audio source in baja.
///
/// Combines `Read + Seek + MediaSource` (required by Symphonia) with
/// baja-specific metadata accessors.
pub trait AudioSource: Read + Seek + MediaSource + Send {
    /// MIME / content-type of the stream, if known.
    fn content_type(&self) -> Option<String> {
        None
    }

    /// Whether the source supports seeking (i.e. `Content-Length` is known).
    fn seekable(&self) -> bool {
        self.is_seekable()
    }
}

// ─── create_client ────────────────────────────────────────────────────────────

/// Build a `reqwest::Client` with optional UA, local address and proxy.
///
/// Shared helper used by every source reader — replaces the old
/// `audio::remote_reader::create_client`.
pub fn create_client(
    user_agent: String,
    local_addr: Option<std::net::IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
    headers: Option<reqwest::header::HeaderMap>,
) -> crate::common::types::AnyResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs(15));

    if let Some(headers) = headers {
        builder = builder.default_headers(headers);
    }
    if let Some(ip) = local_addr {
        builder = builder.local_address(ip);
    }
    if let Some(proxy_config) = proxy {
        if let Some(p_url) = &proxy_config.url {
            if let Ok(mut proxy_obj) = reqwest::Proxy::all(p_url) {
                if let (Some(u), Some(p)) = (proxy_config.username, proxy_config.password) {
                    proxy_obj = proxy_obj.basic_auth(&u, &p);
                }
                builder = builder.proxy(proxy_obj);
            }
        }
    }

    Ok(builder.build()?)
}
