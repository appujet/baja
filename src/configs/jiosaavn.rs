use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnConfig {
    pub decryption: Option<JioSaavnDecryptionConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnDecryptionConfig {
    #[serde(rename = "secretKey")]
    pub secret_key: Option<String>,
}
