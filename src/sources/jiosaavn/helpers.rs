use serde_json::Value;
use tracing::warn;

pub const API_BASE: &str = "https://www.jiosaavn.com/api.php";

pub async fn get_json(client: &reqwest::Client, params: &[(&str, &str)]) -> Option<Value> {
    let resp = match client
        .get(API_BASE)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36")
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
            warn!("JioSaavn request failed: {}", e);
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
            warn!("Failed to read response body: {}", e);
            return None;
        }
    };
    serde_json::from_str(&text).ok()
}

pub fn clean_string(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&amp;", "&")
}
