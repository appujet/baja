use std::{net::IpAddr, time::Duration};

use tracing::warn;

use crate::audio::constants::HTTP_CLIENT_TIMEOUT_SECS;

pub fn create_client(
    user_agent: String,
    local_addr: Option<IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
    headers: Option<reqwest::header::HeaderMap>,
) -> crate::common::types::AnyResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(HTTP_CLIENT_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(5))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(25))
        .pool_max_idle_per_host(64)
        .pool_idle_timeout(Duration::from_secs(70))
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
