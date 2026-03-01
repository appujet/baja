use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use reqwest::Client;

use crate::configs::HttpProxyConfig;

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

pub fn default_user_agent() -> String {
    DEFAULT_USER_AGENT.to_string()
}

/// A pool of `reqwest::Client` instances shared across sources.
///
/// Clients are cached by their proxy configuration. This drastically reduces
/// memory usage by sharing connection pools and thread overhead.
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
    ///
    /// If no client exists for this exact proxy, a new one is created and cached.
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
        // Use a generic builder. Specific headers (like different User-Agents)
        // should be applied PER-REQUEST to the shared client.
        let mut builder = Client::builder()
            .user_agent(default_user_agent())
            .cookie_store(true)
            .gzip(true)
            .timeout(Duration::from_secs(15));

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

        builder.build().unwrap_or_else(|_| Client::new())
    }
}

impl Default for HttpClientPool {
    fn default() -> Self {
        Self::new()
    }
}
