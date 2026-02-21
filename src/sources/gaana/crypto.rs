use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use tracing::warn;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

const CRYPTO_KEY: &[u8; 16] = b"gy1t#b@jl(b$wtme";
const CRYPTO_IV: &[u8; 16] = b"xC4dmVJAq14BfntX";
const HLS_BASE_URL: &str = "https://vodhlsgaana-ebw.akamaized.net/";

pub fn decrypt_stream_path(encrypted_data: &str) -> Option<String> {
  if encrypted_data.is_empty() {
    return None;
  }

  let offset = encrypted_data.chars().next()?.to_digit(10)? as usize;

  let skip = offset + 16;
  if skip >= encrypted_data.len() {
    warn!(
      "Gaana: encrypted data too short (len={}, skip={})",
      encrypted_data.len(),
      skip
    );
    return None;
  }

  let ciphertext_b64 = &encrypted_data[skip..];

  let padded = format!(
    "{}{}",
    ciphertext_b64,
    &"==="[..(4 - ciphertext_b64.len() % 4) % 4]
  );

  let ciphertext = match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &padded)
  {
    Ok(data) => data,
    Err(e) => {
      warn!("Gaana: base64 decode failed: {}", e);
      return None;
    }
  };

  if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
    warn!("Gaana: invalid ciphertext length: {}", ciphertext.len());
    return None;
  }

  let mut buf = ciphertext;
  let cipher = Aes128CbcDec::new_from_slices(CRYPTO_KEY, CRYPTO_IV).ok()?;

  let decrypted = match cipher.decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buf)
  {
    Ok(d) => d,
    Err(e) => {
      warn!("Gaana: AES decryption failed: {}", e);
      return None;
    }
  };

  let raw_text: String = decrypted
    .iter()
    .filter(|&&b| b >= 32 && b <= 126)
    .map(|&b| b as char)
    .collect();

  if let Some(idx) = raw_text.find("hls/") {
    let path = &raw_text[idx..];
    Some(format!("{}{}", HLS_BASE_URL, path))
  } else {
    warn!("Gaana: no /hls/ path found in decrypted text");
    None
  }
}
