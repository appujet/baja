use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MirrorsConfig {
    #[serde(default)]
    pub providers: Vec<String>,
}
