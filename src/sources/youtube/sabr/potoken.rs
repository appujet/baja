use rustypipe_botguard::Botguard;
use tracing::debug;

pub struct PoTokenManager;

const PO_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

impl PoTokenManager {
    /// Generate a BotGuard PoToken using `rustypipe-botguard`.
    ///
    /// Flow:
    /// 1. Fetch visitor data from YouTube HTML (if not provided)
    /// 2. Spin up `Botguard::builder().init()` to mint a valid content PoToken
    pub async fn generate_botguard_token(
        video_id: &str,
        visitor_data: Option<&str>,
    ) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        debug!(
            "Generating BotGuard PoToken via rustypipe-botguard for video: {}",
            video_id
        );

        let client = reqwest::Client::builder()
            .user_agent(PO_USER_AGENT)
            .build()?;

        // Step 1: Get visitor data
        let final_visitor_data = match visitor_data {
            Some(vd) if !vd.is_empty() => vd.to_string(),
            _ => Self::fetch_visitor_data(&client).await?,
        };
        debug!("Visitor data: {} chars", final_visitor_data.len());

        // Step 2: Mint PoToken using rustypipe-botguard
        let mut bg = Botguard::builder()
            .user_agent_opt(Some(PO_USER_AGENT))
            .init()
            .await
            .map_err(|e| format!("Failed to init Botguard: {}", e))?;

        let po_token = bg
            .mint_token(video_id)
            .await
            .map_err(|e| format!("Failed to mint PoToken: {}", e))?;

        debug!(
            "BotGuard PoToken generated successfully. token_len={}, visitor_data_len={}",
            po_token.len(),
            final_visitor_data.len()
        );

        Ok((po_token, final_visitor_data))
    }

    /// Fetch VISITOR_DATA from YouTube's homepage HTML.
    async fn fetch_visitor_data(
        client: &reqwest::Client,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let html = client
            .get("https://www.youtube.com")
            .send()
            .await?
            .text()
            .await?;

        let marker = "\"VISITOR_DATA\":\"";
        if let Some(start) = html.find(marker) {
            let from = start + marker.len();
            if let Some(end) = html[from..].find('"') {
                return Ok(html[from..from + end].to_string());
            }
        }

        Err("Could not find visitorData in YouTube HTML".into())
    }
}
