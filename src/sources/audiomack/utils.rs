use hmac::{Hmac, Mac};
use sha1::Sha1;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::collections::BTreeMap;

const CONSUMER_KEY: &str = "audiomack-web";
const CONSUMER_SECRET: &str = "bd8a07e9f23fbe9d808646b730f89b8e";

type HmacSha1 = Hmac<Sha1>;

pub fn percent_encode(s: &str) -> String {
    // RFC 3986 percent encoding: encode everything except alpha, digit, '-', '.', '_', '~'
    // urlencoding::encode is a bit too loose for some OAuth implementations, 
    // so we handle the strict requirement here.
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(*b as char),
            b => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub fn build_auth_header(method: &str, url: &str, params: &BTreeMap<String, String>, nonce: &str, timestamp: &str) -> String {
    let mut oauth_params = BTreeMap::new();
    oauth_params.insert("oauth_consumer_key".to_string(), CONSUMER_KEY.to_string());
    oauth_params.insert("oauth_nonce".to_string(), nonce.to_string());
    oauth_params.insert("oauth_signature_method".to_string(), "HMAC-SHA1".to_string());
    oauth_params.insert("oauth_timestamp".to_string(), timestamp.to_string());
    oauth_params.insert("oauth_version".to_string(), "1.0".to_string());

    let mut all_params = oauth_params.clone();
    for (k, v) in params {
        all_params.insert(percent_encode(k), percent_encode(v));
    }

    let param_string = all_params.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<String>>()
        .join("&");

    let base_string = format!(
        "{}&{}&{}",
        percent_encode(&method.to_uppercase()),
        percent_encode(url),
        percent_encode(&param_string)
    );

    let signing_key = format!("{}&", percent_encode(CONSUMER_SECRET));
    let mut mac = HmacSha1::new_from_slice(signing_key.as_bytes()).unwrap();
    mac.update(base_string.as_bytes());
    let signature = STANDARD.encode(mac.finalize().into_bytes());

    oauth_params.insert("oauth_signature".to_string(), signature);

    let header_parts: Vec<String> = oauth_params.iter()
        .map(|(k, v)| format!("{}=\"{}\"", percent_encode(k), percent_encode(v)))
        .collect();

    format!("OAuth {}", header_parts.join(", "))
}
