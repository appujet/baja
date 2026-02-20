use std::time::Duration;

use reqwest::{Client, Error, blocking};

pub struct HttpClient;

impl HttpClient {
    pub fn random_user_agent() -> String {
        rand_agents::user_agent()
    }

    pub fn new() -> Result<Client, Error> {
        Client::builder()
            .user_agent(Self::random_user_agent())
            .timeout(Duration::from_secs(10))
            .build()
    }

    pub fn new_blocking() -> Result<blocking::Client, Error> {
        blocking::Client::builder()
            .user_agent(Self::random_user_agent())
            .timeout(Duration::from_secs(10)) // 10s timeout
            .build()
    }
}
