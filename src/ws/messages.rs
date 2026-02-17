use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Debug)]
#[serde(tag = "op")]
#[serde(rename_all = "camelCase")]
pub enum IncomingMessage {
    VoiceUpdate {
        guild_id: String,
        session_id: String,
        channel_id: Option<String>,
        event: Value,
    },
    Play {
        guild_id: String,
        track: String,
    },
    Stop {
        guild_id: String,
    },
    Destroy {
        guild_id: String,
    },
}
