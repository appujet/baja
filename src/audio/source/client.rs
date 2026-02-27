use std::net::IpAddr;
use crate::audio::constants::HTTP_CLIENT_TIMEOUT_SECS;

/// Build a `reqwest::Client` with optional UA, local address and proxy.
pub fn create_client(
    user_agent: String,
    local_addr: Option<IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
    headers: Option<reqwest::header::HeaderMap>,
) -> crate::common::types::AnyResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs(HTTP_CLIENT_TIMEOUT_SECS));

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
