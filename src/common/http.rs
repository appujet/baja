use std::time::Duration;

use reqwest::{Client, Error, blocking};

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

pub struct HttpClient;

impl HttpClient {
  pub fn default_user_agent() -> String {
    DEFAULT_USER_AGENT.to_string()
  }

  pub fn new() -> Result<Client, Error> {
    Client::builder()
      .user_agent(Self::default_user_agent())
      .timeout(Duration::from_secs(10))
      .build()
  }

  pub fn new_blocking() -> Result<blocking::Client, Error> {
    blocking::Client::builder()
      .user_agent(Self::default_user_agent())
      .timeout(Duration::from_secs(10)) // 10s timeout
      .build()
  }
}
