use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub version: Version,
    pub build_time: u64,
    pub git: GitInfo,
    pub jvm: String,
    pub lavaplayer: String,
    pub source_managers: Vec<String>,
    pub filters: Vec<String>,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub semver: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub branch: String,
    pub commit: String,
    pub commit_time: u64,
}

#[derive(Debug, Serialize)]
pub struct Plugin {
    pub name: String,
    pub version: String,
}
