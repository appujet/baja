use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use md5::{Digest, Md5};
use tracing::warn;

type BlowfishCbc = cbc::Decryptor<blowfish::Blowfish>;

pub const CHUNK_SIZE: usize = 2048;

/// Handles the specialized MD5-MD5-Master XOR decryption for Deezer chunks.
pub struct DeezerCrypt {
  key: [u8; 16],
}

impl DeezerCrypt {
  pub fn new(track_id: &str, master_key: &str) -> Self {
    let hash = Md5::digest(track_id.as_bytes());
    let hash_hex = hex::encode(hash);
    let hash_bytes = hash_hex.as_bytes();
    let master_bytes = master_key.as_bytes();

    let mut key = [0u8; 16];
    for i in 0..16 {
      key[i] = hash_bytes[i] ^ hash_bytes[i + 16] ^ master_bytes[i];
    }
    Self { key }
  }

  pub fn decrypt_chunk(&self, chunk_index: u64, chunk: &[u8], dest: &mut Vec<u8>) {
    if chunk_index % 3 == 0 {
      let iv = [0, 1, 2, 3, 4, 5, 6, 7];
      let mut buffer = [0u8; CHUNK_SIZE];
      let len = std::cmp::min(chunk.len(), CHUNK_SIZE);
      buffer[..len].copy_from_slice(&chunk[..len]);

      if let Ok(cipher) = BlowfishCbc::new_from_slices(&self.key, &iv) {
        if let Ok(decrypted) =
          cipher.decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buffer)
        {
          dest.extend_from_slice(decrypted);
          return;
        } else {
          warn!(
            "Blowfish decryption failed for chunk {}, falling back to raw",
            chunk_index
          );
        }
      }
    }
    dest.extend_from_slice(chunk);
  }
}
