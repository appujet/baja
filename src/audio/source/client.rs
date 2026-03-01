use std::{net::IpAddr, time::Duration};

use tracing::warn;

use crate::audio::constants::HTTP_CLIENT_TIMEOUT_SECS;

/// Build a `reqwest::Client` with optimal settings for high-throughput audio
/// streaming:
///
/// - Separate connect vs. total timeout so a slow handshake does not eat the
///   entire request budget.
/// - `tcp_nodelay` disables Nagle's algorithm, reducing latency for large chunk
///   writes.
/// - `tcp_keepalive` surfaces dead connections before they are reused.
/// - `pool_max_idle_per_host` / `pool_idle_timeout` prevent stale-connection
///   failures while keeping memory bounded.
/// - HTTP/2 adaptive window enables multiplexed chunk downloading on servers
///   that support it.
///
/// `cookie_store` is intentionally disabled: session cookies are not needed
/// and enabling it would allow unbounded memory growth with many unique hosts.
pub fn create_client(
    user_agent: String,
    local_addr: Option<IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
    headers: Option<reqwest::header::HeaderMap>,
) -> crate::common::types::AnyResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent)
        // Total request timeout (headers + body).
        .timeout(Duration::from_secs(HTTP_CLIENT_TIMEOUT_SECS))
        // Separate connect timeout: a slow handshake must not eat the full budget.
        .connect_timeout(Duration::from_secs(5))
        // Disable Nagle's algorithm for low-latency chunk streaming.
        .tcp_nodelay(true)
        // Keep-alive probes surface dead connections before they are reused.
        .tcp_keepalive(Duration::from_secs(25))
        // Allow up to 64 idle connections per host (audio origin servers).
        .pool_max_idle_per_host(64)
        // Evict idle connections after 70 s to avoid stale-connection resets.
        .pool_idle_timeout(Duration::from_secs(70))
        // HTTP/2 adaptive flow-control window for sources that support it.
        .http2_adaptive_window(true);

    if let Some(headers) = headers {
        builder = builder.default_headers(headers);
    }
    if let Some(ip) = local_addr {
        builder = builder.local_address(ip);
    }
    if let Some(proxy_config) = proxy {
        if let Some(p_url) = &proxy_config.url {
            match reqwest::Proxy::all(p_url) {
                Ok(mut proxy_obj) => {
                    if let (Some(u), Some(p)) = (proxy_config.username, proxy_config.password) {
                        proxy_obj = proxy_obj.basic_auth(&u, &p);
                    }
                    builder = builder.proxy(proxy_obj);
                }
                Err(e) => {
                    warn!(
                        "Failed to parse proxy URL '{}': {} — proxy will be ignored",
                        p_url, e
                    );
                }
            }
        }
    }

    Ok(builder.build()?)
}
