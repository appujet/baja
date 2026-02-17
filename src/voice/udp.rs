use davey::{AeadInPlace as AesAeadInPlace, Aes256Gcm, KeyInit as AesKeyInit};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use xsalsa20poly1305::XSalsa20Poly1305;
use xsalsa20poly1305::aead::Aead as SalsaAead;

pub enum EncryptionMode {
    XSalsa20Poly1305,
    Aes256Gcm,
}

pub struct UdpBackend {
    socket: UdpSocket,
    ssrc: u32,
    address: std::net::SocketAddr,
    mode: EncryptionMode,
    salsa_cipher: Option<XSalsa20Poly1305>,
    aes_cipher: Option<Aes256Gcm>,

    // Internal state management to match NodeLink's Connection class
    sequence: AtomicU16,
    timestamp: AtomicU32,
    nonce: AtomicU32,
}

impl UdpBackend {
    pub fn new(
        socket: UdpSocket,
        address: std::net::SocketAddr,
        ssrc: u32,
        secret_key: [u8; 32],
        mode_name: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mode = match mode_name {
            "aead_aes256_gcm_rtpsize" => EncryptionMode::Aes256Gcm,
            _ => EncryptionMode::XSalsa20Poly1305,
        };

        let mut salsa_cipher = None;
        let mut aes_cipher = None;

        match mode {
            EncryptionMode::XSalsa20Poly1305 => {
                salsa_cipher = Some(XSalsa20Poly1305::new(&secret_key.into()));
            }
            EncryptionMode::Aes256Gcm => {
                aes_cipher = Some(Aes256Gcm::new(&secret_key.into()));
            }
        }

        Ok(Self {
            socket,
            ssrc,
            address,
            mode,
            salsa_cipher,
            aes_cipher,
            sequence: AtomicU16::new(0),
            timestamp: AtomicU32::new(0),
            nonce: AtomicU32::new(0),
        })
    }

    pub fn send_opus_packet(&self, payload: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        // Increment sequence and timestamp like NodeLink
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        let timestamp = self.timestamp.fetch_add(960, Ordering::SeqCst);

        // Nonce is incremented before use in NodeLink
        let current_nonce = self.nonce.fetch_add(1, Ordering::SeqCst).wrapping_add(1);

        let mut header = [0u8; 12];
        header[0] = 0x80; // Version 2
        header[1] = 0x78; // Payload Type (Opus)
        header[2..4].copy_from_slice(&sequence.to_be_bytes());
        header[4..8].copy_from_slice(&timestamp.to_be_bytes());
        header[8..12].copy_from_slice(&self.ssrc.to_be_bytes());

        match self.mode {
            EncryptionMode::XSalsa20Poly1305 => {
                let mut nonce = [0u8; 24];
                // For xsalsa20_poly1305, Discord expects the header as the first 12 bytes
                nonce[0..12].copy_from_slice(&header);

                let encrypted = self
                    .salsa_cipher
                    .as_ref()
                    .unwrap()
                    .encrypt(&nonce.into(), payload)
                    .map_err(|e| format!("Salsa encryption error: {:?}", e))?;

                let mut packet = Vec::with_capacity(12 + encrypted.len());
                packet.extend_from_slice(&header);
                packet.extend_from_slice(&encrypted);

                self.socket.send_to(&packet, self.address)?;
            }
            EncryptionMode::Aes256Gcm => {
                let counter_bytes = current_nonce.to_be_bytes();

                let mut nonce_bytes = [0u8; 12];
                // NodeLink writes the 4-byte counter to the first 4 bytes of the 12-byte nonce
                nonce_bytes[0..4].copy_from_slice(&counter_bytes);

                let mut buffer = payload.to_vec();
                let tag = self
                    .aes_cipher
                    .as_ref()
                    .unwrap()
                    .encrypt_in_place_detached(&nonce_bytes.into(), &header, &mut buffer)
                    .map_err(|e| format!("AES-GCM encryption error: {:?}", e))?;

                let mut packet = Vec::with_capacity(12 + buffer.len() + 16 + 4);
                packet.extend_from_slice(&header);
                packet.extend_from_slice(&buffer);
                packet.extend_from_slice(&tag);
                packet.extend_from_slice(&counter_bytes);

                self.socket.send_to(&packet, self.address)?;
            }
        }
        Ok(())
    }
}
