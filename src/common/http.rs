use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use reqwest::Client;
use tracing::warn;

use crate::configs::HttpProxyConfig;

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

pub fn default_user_agent() -> String {
    DEFAULT_USER_AGENT.to_string()
}

/// A pool of `reqwest::Client` instances shared across sources.
pub struct HttpClientPool {
    clients: DashMap<Option<HttpProxyConfig>, Arc<Client>>,
}

impl HttpClientPool {
    pub fn new() -> Self {
        Self {
            clients: DashMap::new(),
        }
    }

    /// Get a shared client for the given proxy configuration.
    pub fn get(&self, proxy: Option<HttpProxyConfig>) -> Arc<Client> {
        if let Some(client) = self.clients.get(&proxy) {
            return client.clone();
        }

        // Slow path: create and insert
        let client = Arc::new(self.create_client(proxy.clone()));
        self.clients.insert(proxy, client.clone());
        client
    }

    fn create_client(&self, proxy: Option<HttpProxyConfig>) -> Client {
        let mut builder = Client::builder()
            .user_agent(default_user_agent())
            .gzip(true)
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(25))
            .pool_idle_timeout(Duration::from_secs(70))
            .http2_adaptive_window(true);

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
                            "HttpClientPool: failed to parse proxy URL '{}': {} — proxy will be ignored",
                            p_url, e
                        );
                    }
                }
            }
        }

        match builder.build() {
            Ok(client) => client,
            Err(e) => {
                warn!(
                    "HttpClientPool: failed to build client ({}), falling back to default",
                    e
                );
                Client::new()
            }
        }
    }
}

impl Default for HttpClientPool {
    fn default() -> Self {
        Self::new()
    }
}
