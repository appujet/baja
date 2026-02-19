use base64::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct PoTokenManager;

impl PoTokenManager {
    pub async fn fetch_visitor_data() -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let res = client.get("https://www.youtube.com")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36")
            .send()
            .await?;

        let html = res.text().await?;
        let marker = r#""VISITOR_DATA":""#;
        if let Some(start) = html.find(marker) {
            let from = start + marker.len();
            if let Some(end) = html[from..].find('"') {
                return Ok(html[from..from + end].to_string());
            }
        }

        Err("Could not find VISITOR_DATA in YouTube home page".into())
    }
    pub fn generate_cold_start_token(visitor_data: &str) -> String {
        let identifier = visitor_data.as_bytes();

        // Ensure timestamp is valid
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;

        let key0: u8 = rand::random();
        let key1: u8 = rand::random();

        // Packet size: Header (2 bytes) + Payload (8 + identifier.len())
        let mut packet = vec![0u8; 10 + identifier.len()];

        packet[0] = 34; // 0x22
        packet[1] = (8 + identifier.len()) as u8;

        // Payload starts at index 2
        packet[2] = key0;
        packet[3] = key1;
        packet[4] = 0;
        packet[5] = 1;

        packet[6] = (ts >> 24) as u8;
        packet[7] = (ts >> 16) as u8;
        packet[8] = (ts >> 8) as u8;
        packet[9] = ts as u8;

        packet[10..].copy_from_slice(identifier);

        // XOR obfuscation logic
        // Corresponding to JS:
        // const payload = packet.subarray(2)
        // for (let i = 2; i < payload.length; i++) payload[i] ^= payload[i & 1]

        let payload_len = 8 + identifier.len();
        for i in 2..payload_len {
            // i is index inside payload which starts after 2 header bytes
            let packet_idx = 2 + i;
            let key_idx = 2 + (i & 1);
            packet[packet_idx] ^= packet[key_idx];
        }

        BASE64_URL_SAFE_NO_PAD.encode(&packet)
    }
}
