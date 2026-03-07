use std::sync::Arc;
use serde_json::Value;

pub async fn get_json(client: &Arc<reqwest::Client>, url: &str) -> Option<Value> {
    let res = match client.get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
        .send()
        .await {
            Ok(r) => r,
            Err(e) => {
                let redacted = if let Some(pos) = url.find("api_key=") {
                    let end = url[pos..].find('&').map(|e| pos + e).unwrap_or(url.len());
                    let mut s = url.to_owned();
                    s.replace_range(pos + 8..end, "REDACTED");
                    s
                } else {
                    url.to_owned()
                };
                tracing::debug!("Last.fm: API request failed for {}: {}", redacted, e);
                return None;
            }
        };

    if !res.status().is_success() {
        let redacted = if let Some(pos) = url.find("api_key=") {
            let end = url[pos..].find('&').map(|e| pos + e).unwrap_or(url.len());
            let mut s = url.to_owned();
            s.replace_range(pos + 8..end, "REDACTED");
            s
        } else {
            url.to_owned()
        };
        tracing::debug!("Last.fm: API returned error status {} for {}", res.status(), redacted);
        return None;
    }

    res.json().await.ok()
}

pub fn unescape_html(input: &str) -> String {
    let mut result = input.to_owned();
    loop {
        let next = result
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&apos;", "'")
            .replace("&#x27;", "'");
        
        if next == result {
            break;
        }
        result = next;
    }
    result
}

pub fn parse_duration_to_ms(duration: &str) -> u64 {
    let parts: Vec<&str> = duration.split(':').collect();
    if parts.len() == 2 {
        let minutes = parts[0].trim().parse::<u64>().unwrap_or(0);
        let seconds = parts[1].trim().parse::<u64>().unwrap_or(0);
        (minutes * 60 + seconds) * 1000
    } else if parts.len() == 3 {
        let hours = parts[0].trim().parse::<u64>().unwrap_or(0);
        let minutes = parts[1].trim().parse::<u64>().unwrap_or(0);
        let seconds = parts[2].trim().parse::<u64>().unwrap_or(0);
        (hours * 3600 + minutes * 60 + seconds) * 1000
    } else {
        0
    }
}
