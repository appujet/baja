use std::{net::IpAddr, time::Duration};

use reqwest::{Client, Proxy, header::HeaderMap};
use tracing::warn;

use crate::{common::types::AnyResult, config::HttpProxyConfig};

pub fn create_client(
    user_agent: String,
    local_addr: Option<IpAddr>,
    proxy: Option<HttpProxyConfig>,
    headers: Option<HeaderMap>,
) -> AnyResult<Client> {
    let mut builder = Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(8))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(25))
        .pool_max_idle_per_host(64)
        .pool_idle_timeout(Duration::from_secs(70));

    if let Some(headers) = headers {
        builder = builder.default_headers(headers);
    }

    if let Some(ip) = local_addr {
        builder = builder.local_address(ip);
    }

    if let Some(p_cfg) = proxy
        && let Some(p_url) = p_cfg.url
    {
        match Proxy::all(&p_url) {
            Ok(mut p) => {
                if let (Some(u), Some(pw)) = (p_cfg.username, p_cfg.password) {
                    p = p.basic_auth(&u, &pw);
                }
                builder = builder.proxy(p);
            }
            Err(e) => warn!("Failed to parse proxy URL '{}': {}", p_url, e),
        }
    }

    Ok(builder.build()?)
}
