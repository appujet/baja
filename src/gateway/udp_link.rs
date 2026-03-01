use std::{net::SocketAddr, sync::Arc};

use davey::{AeadInPlace as AesAeadInPlace, Aes256Gcm, KeyInit as AesKeyInit};
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

/// Encryption cipher — exactly one is active at any given time.
///
/// Using an enum (rather than `Option<Salsa> + Option<Aes>`) makes the
/// impossible "both-initialized" or "neither-initialized" states
/// unrepresentable at the type level.
enum ActiveCipher {
    XSalsa20Poly1305(XSalsa20Poly1305),
    Aes256Gcm(Aes256Gcm),
}

pub struct UdpBackend {
    socket: Arc<tokio::net::UdpSocket>,
    ssrc: u32,
    address: SocketAddr,
    cipher: ActiveCipher,
    sequence: u16,
    timestamp: u32,
    nonce: u32,
    /// Reusable packet buffer — allocated once, cleared per frame.
    packet_buf: Vec<u8>,
}

impl UdpBackend {
    pub fn new(
        socket: Arc<tokio::net::UdpSocket>,
        address: SocketAddr,
        ssrc: u32,
        secret_key: [u8; 32],
        mode_name: &str,
    ) -> AnyResult<Self> {
        let cipher = match mode_name {
            "aead_aes256_gcm_rtpsize" => {
                ActiveCipher::Aes256Gcm(Aes256Gcm::new(&secret_key.into()))
            }
            _ => ActiveCipher::XSalsa20Poly1305(XSalsa20Poly1305::new(&secret_key.into())),
        };

        Ok(Self {
            socket,
            ssrc,
            address,
            cipher,
            sequence: 0,
            timestamp: 0,
            nonce: 0,
            packet_buf: Vec::with_capacity(UDP_PACKET_BUF_CAPACITY),
        })
    }

    pub async fn send_opus_packet(&mut self, payload: &[u8]) -> AnyResult<()> {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);

        let timestamp = self.timestamp;
        self.timestamp = self.timestamp.wrapping_add(RTP_TIMESTAMP_STEP);

        self.nonce = self.nonce.wrapping_add(1);
        let current_nonce = self.nonce;

        // Build the 12-byte RTP header.
        let mut header = [0u8; 12];
        header[0] = RTP_VERSION_BYTE;
        header[1] = RTP_OPUS_PAYLOAD_TYPE;
        header[2..4].copy_from_slice(&sequence.to_be_bytes());
        header[4..8].copy_from_slice(&timestamp.to_be_bytes());
        header[8..12].copy_from_slice(&self.ssrc.to_be_bytes());

        self.packet_buf.clear();
        self.packet_buf.extend_from_slice(&header);
        self.packet_buf.extend_from_slice(payload);

        match &self.cipher {
            ActiveCipher::XSalsa20Poly1305(cipher) => {
                // Discord voice: nonce = RTP header padded to 24 bytes.
                let mut nonce = [0u8; 24];
                nonce[0..12].copy_from_slice(&header);

                let tag = cipher
                    .encrypt_in_place_detached(&nonce.into(), &header, &mut self.packet_buf[12..])
                    .map_err(|e| map_boxed_err(format!("XSalsa20 encryption error: {e:?}")))?;

                self.packet_buf.extend_from_slice(&tag);
            }
            ActiveCipher::Aes256Gcm(cipher) => {
                // Discord AES-GCM: 4-byte little-endian nonce counter in first 4 bytes.
                let mut nonce_bytes = [0u8; 12];
                nonce_bytes[0..4].copy_from_slice(&current_nonce.to_be_bytes());

                let tag = cipher
                    .encrypt_in_place_detached(
                        &nonce_bytes.into(),
                        &header,
                        &mut self.packet_buf[12..],
                    )
                    .map_err(|e| map_boxed_err(format!("AES-GCM encryption error: {e:?}")))?;

                self.packet_buf.extend_from_slice(&tag);
                self.packet_buf
                    .extend_from_slice(&current_nonce.to_be_bytes());
            }
        }

        self.socket
            .send_to(&self.packet_buf, self.address)
            .await
            .map_err(map_boxed_err)?;

        Ok(())
    }
}
