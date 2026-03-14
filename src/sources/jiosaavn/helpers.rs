use serde_json::Value;
use tracing::warn;


const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36";

pub async fn get_json(client: &reqwest::Client, api_url: &str, params: &[(&str, &str)]) -> Option<Value> {
    let resp = match client
        .get(api_url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Referer", "https://www.jiosaavn.com/")
        .header("Origin", "https://www.jiosaavn.com")
        .query(params)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("JioSaavn request failed: {e}");
            return None;
        }
    };

    if !resp.status().is_success() {
        warn!("JioSaavn API error status: {}", resp.status());
        return None;
    }

    let text = match resp.text().await {
        Ok(text) => text,
        Err(e) => {
            warn!("Failed to read JioSaavn response body: {e}");
            return None;
        }
    };

    serde_json::from_str(&text).ok()
}

pub fn clean_string(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&amp;", "&")
}
