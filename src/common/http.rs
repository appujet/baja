use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use reqwest::{Client, Proxy};
use tracing::warn;

use crate::{common::utils::default_user_agent, config::HttpProxyConfig};

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
        self.clients
            .entry(proxy.clone())
            .or_insert_with(|| Arc::new(self.create_client(proxy)))
            .clone()
    }

    fn create_client(&self, proxy: Option<HttpProxyConfig>) -> Client {
        let mut builder = Client::builder()
            .user_agent(default_user_agent())
            .gzip(true)
            .deflate(true)
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .tcp_nodelay(true)
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(70));

        if let Some(url) = proxy.as_ref().and_then(|config| config.url.as_ref()) {
            match Proxy::all(url) {
                Ok(mut proxy_obj) => {
                    if let Some((u, p)) = proxy
                        .as_ref()
                        .and_then(|c| c.username.as_ref().zip(c.password.as_ref()))
                    {
                        proxy_obj = proxy_obj.basic_auth(u, p);
                    }
                    builder = builder.proxy(proxy_obj);
                }
                Err(e) => {
                    warn!(
                        "HttpClientPool: failed to parse proxy URL '{}': {} — proxy will be ignored",
                        url, e
                    );
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
