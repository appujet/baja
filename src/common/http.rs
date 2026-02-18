use reqwest::{blocking, Client, Error};
use std::time::Duration;

pub struct HttpClient;

impl HttpClient {
    pub const USER_AGENT: &'static str = "Mozilla/5.0 (compatible; Baja/0.1.0)";

    pub fn new() -> Result<Client, Error> {
        Client::builder()
            .user_agent(Self::USER_AGENT)
            .timeout(Duration::from_secs(10))
            .build()
    }

    pub fn new_blocking() -> Result<blocking::Client, Error> {
        blocking::Client::builder()
            .user_agent(Self::USER_AGENT)
            .timeout(Duration::from_secs(10)) // 10s timeout
            .build()
    }
}
