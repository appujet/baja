use crate::configs::HttpProxyConfig;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use std::net::IpAddr;
use std::sync::Arc;

/// Configuration and contextual data for a decoder session.
#[derive(Clone)]
pub struct DecoderContext {
    pub url: String,
    pub local_addr: Option<IpAddr>,
    pub cipher_manager: Option<Arc<YouTubeCipherManager>>,
    pub proxy: Option<HttpProxyConfig>,
}

impl DecoderContext {
    pub fn new(
        url: String,
        local_addr: Option<IpAddr>,
        cipher_manager: Option<Arc<YouTubeCipherManager>>,
        proxy: Option<HttpProxyConfig>,
    ) -> Self {
        Self {
            url,
            local_addr,
            cipher_manager,
            proxy,
        }
    }
}
