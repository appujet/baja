use std::time::Duration;

use reqwest::{Client, Error};

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

pub fn default_user_agent() -> String {
  DEFAULT_USER_AGENT.to_string()
}

pub fn new_client() -> Result<Client, Error> {
  Client::builder()
    .user_agent(default_user_agent())
    .timeout(Duration::from_secs(10))
    .build()
}
