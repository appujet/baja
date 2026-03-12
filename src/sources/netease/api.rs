use aes::{
    Aes128,
    cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray},
};
use md5::{Digest, Md5};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use tracing::{debug, error};

#[derive(Debug, Deserialize)]
pub struct NeteaseResponse<T> {
    pub code: i64,
    #[serde(flatten)]
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub struct NeteaseArtist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct NeteaseAlbum {
    pub id: i64,
    pub name: String,
    #[serde(alias = "picUrl")]
    pub pic_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NeteaseSong {
    pub id: i64,
    pub name: String,
    #[serde(alias = "ar", alias = "artists")]
    pub artists: Vec<NeteaseArtist>,
    #[serde(alias = "al", alias = "album")]
    pub album: Option<NeteaseAlbum>,
    #[serde(alias = "dt", alias = "duration")]
    pub duration: u64,
}

#[derive(Debug, Deserialize)]
pub struct SearchResultData {
    pub result: SearchResultInner,
}

#[derive(Debug, Deserialize)]
pub struct SearchResultInner {
    #[serde(default)]
    pub songs: Vec<NeteaseSong>,
    #[serde(default)]
    pub albums: Vec<NeteaseAlbum>,
    #[serde(default)]
    pub artists: Vec<NeteaseArtist>,
    #[serde(default)]
    pub playlists: Vec<NeteasePlaylist>,
}

#[derive(Debug, Deserialize)]
pub struct NeteasePlaylist {
    pub id: i64,
    pub name: String,
    #[serde(alias = "coverImgUrl")]
    pub cover_img_url: Option<String>,
    pub creator: Option<NeteaseCreator>,
    #[serde(alias = "trackCount")]
    pub track_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct NeteaseCreator {
    pub nickname: String,
}

#[derive(Debug, Deserialize)]
pub struct SongDetailData {
    pub songs: Vec<NeteaseSong>,
}

#[derive(Debug, Deserialize)]
pub struct TrackUrlData {
    pub data: Vec<TrackUrlItem>,
}

#[derive(Debug, Deserialize)]
pub struct TrackUrlItem {
    pub id: i64,
    pub url: Option<String>,
    pub br: i64,
    pub code: i64,
    #[serde(rename = "freeTrialInfo")]
    pub free_trial_info: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SimilarSongsData {
    pub songs: Vec<NeteaseSong>,
}

#[derive(Debug, Deserialize)]
pub struct AlbumDetailData {
    pub album: NeteaseAlbum,
    pub songs: Vec<NeteaseSong>,
}

#[derive(Debug, Deserialize)]
pub struct PlaylistDetailData {
    pub playlist: PlaylistInfo,
}

#[derive(Debug, Deserialize)]
pub struct PlaylistInfo {
    pub id: i64,
    pub name: String,
    pub tracks: Vec<NeteaseSong>,
}

#[derive(Debug, Deserialize)]
pub struct ArtistDetailData {
    pub artist: NeteaseArtist,
    #[serde(rename = "hotSongs")]
    pub hot_songs: Vec<NeteaseSong>,
}

const EAPI_KEY: &[u8] = b"e82ckenh8dichen8";
const EAPI_URLS: &[&str] = &[
    "https://interface3.music.163.com/eapi",
    "https://interface.music.163.com/eapi",
];

pub fn aes_encrypt_ecb(data: &[u8], key: &[u8]) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut padded_data = pkcs7_pad(data, 16);
    for chunk in padded_data.chunks_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }
    padded_data
}

fn pkcs7_pad(data: &[u8], block_size: usize) -> Vec<u8> {
    let padding_len = block_size - (data.len() % block_size);
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat_n(padding_len as u8, padding_len));
    padded
}

pub fn md5_hex(data: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

pub fn eapi_encrypt(url: &str, obj: &Value) -> String {
    let text = serde_json::to_string(obj).unwrap_or_default();
    let message = format!("nobody{}use{}md5forencrypt", url, text);
    let digest = md5_hex(&message);
    let data = format!("{}-36cd479b6b5-{}-36cd479b6b5-{}", url, text, digest);
    let encrypted = aes_encrypt_ecb(data.as_bytes(), EAPI_KEY);
    hex::encode(encrypted).to_uppercase()
}

pub async fn get_eapi_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    path: &str,
    obj: Value,
    nuid: &str,
    device_id: &str,
) -> Option<T> {
    let params = eapi_encrypt(path, &obj);
    let network_path = path.strip_prefix("/api").unwrap_or(path);

    for &base_url in EAPI_URLS {
        let url = format!("{}{}", base_url, network_path);

        let resp = client.post(&url)
            .header("User-Agent", "NeteaseMusic/2.5.1 (iPhone; iOS 16.6; Scale/3.00)")
            .header("Referer", "https://music.163.com/")
            .header("Origin", "https://music.163.com")
            .header("X-Real-IP", "118.88.88.88")
            .header("X-Forwarded-For", "118.88.88.88")
            .header("X-Netease-PC-IP", "118.88.88.88")
            .header("Cookie", format!(
                "os=iOS; appver=2.5.1; _ntes_nuid={}; deviceId={}; channel=AppStore; mobilename=iPhone15,3", 
                nuid, device_id
            ))
            .form(&[("params", params.clone())])
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let text = r.text().await.ok()?;
                match serde_json::from_str::<NeteaseResponse<T>>(&text) {
                    Ok(res) => {
                        if res.code == 200 || res.code == 0 {
                            return Some(res.data);
                        } else {
                            debug!(
                                "Netease API {} returned application code {}: {}",
                                path, res.code, text
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            "Netease API {} failed to parse JSON: {}. Text: {}",
                            path, e, text
                        );
                    }
                }
            }
            _ => continue,
        }
    }

    None
}
