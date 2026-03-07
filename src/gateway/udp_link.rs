use std::{net::SocketAddr, sync::Arc};

use davey::{AeadInPlace, Aes256Gcm, KeyInit};
use tokio::net::UdpSocket;
use xsalsa20poly1305::XSalsa20Poly1305;

use crate::{
    common::types::AnyResult,
    gateway::{
        constants::{
            RTP_OPUS_PAYLOAD_TYPE, RTP_TIMESTAMP_STEP, RTP_VERSION_BYTE, UDP_PACKET_BUF_CAPACITY,
        },
        session::types::map_boxed_err,
    },
};

/// Handles RTP encryption and packet construction for Discord voice.
pub struct VoiceTransport {
    socket: Arc<UdpSocket>,
    address: SocketAddr,
    ssrc: u32,
    crypto: CryptoBackend,
    rtp: RtpState,
    buffer: Vec<u8>,
}

enum CryptoBackend {
    XSalsa20Poly1305(Box<XSalsa20Poly1305>),
    Aes256Gcm(Box<Aes256Gcm>),
}

struct RtpState {
    sequence: u16,
    timestamp: u32,
    nonce: u32,
}

impl VoiceTransport {
    pub fn new(
        socket: Arc<UdpSocket>,
        address: SocketAddr,
        ssrc: u32,
        secret_key: [u8; 32],
        mode: &str,
    ) -> AnyResult<Self> {
        let crypto = match mode {
            "aead_aes256_gcm_rtpsize" => {
                CryptoBackend::Aes256Gcm(Box::new(Aes256Gcm::new(&secret_key.into())))
            }
            _ => {
                CryptoBackend::XSalsa20Poly1305(Box::new(XSalsa20Poly1305::new(&secret_key.into())))
            }
        };

        Ok(Self {
            socket,
            address,
            ssrc,
            crypto,
            rtp: RtpState::randomize(),
            buffer: Vec::with_capacity(UDP_PACKET_BUF_CAPACITY),
        })
    }

    pub async fn send_keepalive(&self) -> AnyResult<()> {
        let mut keepalive = [0u8; 9];
        keepalive[0..4].copy_from_slice(&self.ssrc.to_be_bytes());
        self.socket.send_to(&keepalive, self.address).await?;
        Ok(())
    }

    pub async fn transmit_opus(&mut self, opus_data: &[u8]) -> AnyResult<()> {
        let (seq, ts, nonce_val) = self.rtp.next();

        // Build RTP Header (12 bytes)
        let mut header = [0u8; 12];
        header[0] = RTP_VERSION_BYTE;
        header[1] = RTP_OPUS_PAYLOAD_TYPE;
        header[2..4].copy_from_slice(&seq.to_be_bytes());
        header[4..8].copy_from_slice(&ts.to_be_bytes());
        header[8..12].copy_from_slice(&self.ssrc.to_be_bytes());

        self.buffer.clear();
        self.buffer.extend_from_slice(&header);
        self.buffer.extend_from_slice(opus_data);

        match &self.crypto {
            CryptoBackend::XSalsa20Poly1305(cipher) => {
                let mut nonce = [0u8; 24];
                nonce[0..12].copy_from_slice(&header);

                let tag = cipher
                    .encrypt_in_place_detached(&nonce.into(), &header, &mut self.buffer[12..])
                    .map_err(|e| map_boxed_err(format!("XSalsa20 error: {e:?}")))?;

                self.buffer.extend_from_slice(&tag);
            }
            CryptoBackend::Aes256Gcm(cipher) => {
                let mut nonce = [0u8; 12];
                nonce[0..4].copy_from_slice(&nonce_val.to_be_bytes());

                let tag = cipher
                    .encrypt_in_place_detached(&nonce.into(), &header, &mut self.buffer[12..])
                    .map_err(|e| map_boxed_err(format!("AES-GCM error: {e:?}")))?;

                self.buffer.extend_from_slice(&tag);
                self.buffer.extend_from_slice(&nonce_val.to_be_bytes());
            }
        }

        self.socket.send_to(&self.buffer, self.address).await?;
        Ok(())
    }
}

impl RtpState {
    fn randomize() -> Self {
        Self {
            sequence: rand::random(),
            timestamp: rand::random(),
            nonce: rand::random(),
        }
    }

    fn next(&mut self) -> (u16, u32, u32) {
        let seq = self.sequence;
        let ts = self.timestamp;
        let n = self.nonce;

        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(RTP_TIMESTAMP_STEP);
        self.nonce = self.nonce.wrapping_add(1);

        (seq, ts, n)
    }
}
