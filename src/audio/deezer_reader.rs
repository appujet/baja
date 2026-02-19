use blowfish::Blowfish;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use crate::audio::reader::RemoteReader;
use md5::{Md5, Digest};
use std::io::{Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;
use hex;

type BlowfishCbc = cbc::Decryptor<blowfish::Blowfish>;

pub struct DeezerReader {
    reader: RemoteReader,
    key: [u8; 16],
    pos: u64,
    overflow_buf: Vec<u8>, // Stores raw bytes read from remote but not yet enough for a chunk
    decrypted_buf: Vec<u8>, // Stores decrypted bytes ready to be read by consumer
    skip_bytes: usize, // Bytes to skip from the beginning of the next decrypted chunk (for seeking)
}

impl DeezerReader {
    pub fn new(
        url: &str,
        track_id: &str,
        master_key: &str,
        local_addr: Option<std::net::IpAddr>,
        proxy: Option<crate::configs::HttpProxyConfig>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let reader = RemoteReader::new(url, local_addr, proxy)?;
        let key = Self::compute_key(track_id, master_key);
        
        Ok(Self {
            reader,
            key,
            pos: 0,
            overflow_buf: Vec::with_capacity(4096),
            decrypted_buf: Vec::with_capacity(4096),
            skip_bytes: 0,
        })
    }

    fn compute_key(track_id: &str, master_key: &str) -> [u8; 16] {
        let md5_hash = Md5::digest(track_id.as_bytes());
        let md5_hex = hex::encode(md5_hash); // hex string of the hash
        let mut key = [0u8; 16];
        let master_bytes = master_key.as_bytes();
        let md5_bytes = md5_hex.as_bytes(); // bytes of the hex string

        for i in 0..16 {
            key[i] = md5_bytes[i] ^ md5_bytes[i + 16] ^ master_bytes[i];
        }
        
        key
    }
}

impl Read for DeezerReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            // 1. Drain skip_bytes if needed
            if self.skip_bytes > 0 && !self.decrypted_buf.is_empty() {
                let to_skip = std::cmp::min(self.skip_bytes, self.decrypted_buf.len());
                self.decrypted_buf.drain(..to_skip);
                self.skip_bytes -= to_skip;
            }

            // 2. Serve from decrypted buffer if available (and skip_bytes satisfied)
            if self.skip_bytes == 0 && !self.decrypted_buf.is_empty() {
                let len = std::cmp::min(buf.len(), self.decrypted_buf.len());
                buf[..len].copy_from_slice(&self.decrypted_buf[..len]);
                self.decrypted_buf.drain(..len);
                return Ok(len);
            }

            // 3. Read from remote into overflow buffer until we have at least 2048 bytes or EOF
            let mut temp_buf = [0u8; 4096]; 
            let mut read_something = false;
            
            // Try to fill overflow_buf to at least 2048
            while self.overflow_buf.len() < 2048 {
                let n = self.reader.read(&mut temp_buf)?;
                if n == 0 {
                    break; // EOF
                }
                self.overflow_buf.extend_from_slice(&temp_buf[..n]);
                read_something = true;
            }
            
            // If we didn't read anything and overflow is empty, we are truly EOF
            if !read_something && self.overflow_buf.is_empty() && self.decrypted_buf.is_empty() {
                return Ok(0);
            }

            // 4. Process chunks from overflow buffer
            while self.overflow_buf.len() >= 2048 {
                 let chunk_data: Vec<u8> = self.overflow_buf.drain(..2048).collect();
                 
                 // Decrypt every 3rd chunk (0, 3, 6...)
                 if (self.pos / 2048) % 3 == 0 {
                    let iv = [0, 1, 2, 3, 4, 5, 6, 7];
                    let cipher = BlowfishCbc::new_from_slices(&self.key, &iv)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                    
                    let mut chunk_mut = chunk_data.clone();
                    if let Ok(_) = cipher.decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut chunk_mut) {
                        self.decrypted_buf.extend_from_slice(&chunk_mut);
                    } else {
                        self.decrypted_buf.extend_from_slice(&chunk_data);
                    }
                 } else {
                     self.decrypted_buf.extend_from_slice(&chunk_data);
                 }
                 self.pos += 2048;
            }

            // If we hit EOF (implied by loop exit without filling 2048) and have leftovers
            // And we know we won't get more data (read returned 0)
            if !read_something && !self.overflow_buf.is_empty() {
                 self.decrypted_buf.extend_from_slice(&self.overflow_buf);
                 self.pos += self.overflow_buf.len() as u64;
                 self.overflow_buf.clear();
            }
            
            // Loop back to start: drain skips, then serve.
        }
    }
}

impl Seek for DeezerReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        // We only support Start(pos) properly for now, or Current(0)
        let target_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(0) => return Ok(self.pos - self.decrypted_buf.len() as u64 - self.overflow_buf.len() as u64), // Approximate
            // Other seeks implementation implies we need to know current size, which is hard.
            // RemoteReader handles it?
            _ => return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Only SeekFrom::Start supported")),
        };

        let aligned_pos = (target_pos / 2048) * 2048;
        let skip = (target_pos - aligned_pos) as usize;

        // Seek the underlying reader to the aligned block start
        let new_pos = self.reader.seek(SeekFrom::Start(aligned_pos))?;
        
        // Reset state
        self.pos = new_pos;
        self.overflow_buf.clear();
        self.decrypted_buf.clear();
        self.skip_bytes = skip; // We need to discard this many bytes from the stream we are about to read
        
        // Return correct logical position
        Ok(target_pos)
    }
}

impl MediaSource for DeezerReader {
    fn is_seekable(&self) -> bool {
        self.reader.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.reader.byte_len()
    }
}
