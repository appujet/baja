use std::{net::SocketAddr, sync::Arc};

use davey::{AeadInPlace, Aes256Gcm, KeyInit};
use serde::{Deserialize, Serialize};
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
pub struct UDPVoiceTransport {
    socket: Arc<UdpSocket>,
    address: SocketAddr,
    pub ssrc: u32,
    pub crypto: CryptoBackend,
    pub rtp: RtpState,
    pub buffer: Vec<u8>,
}

pub enum CryptoBackend {
    XSalsa20Poly1305(Box<XSalsa20Poly1305>),
    Aes256Gcm(Box<Aes256Gcm>),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RtpState {
    pub sequence: u16,
    pub timestamp: u32,
    pub nonce: u32,
}

impl UDPVoiceTransport {
    pub fn new(
        socket: Arc<UdpSocket>,
        address: SocketAddr,
        ssrc: u32,
        secret_key: [u8; 32],
        mode: &str,
        rtp_state: Option<RtpState>,
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
            rtp: rtp_state.unwrap_or_else(RtpState::randomize),
            buffer: Vec::with_capacity(UDP_PACKET_BUF_CAPACITY),
        })
    }

    pub async fn send_keepalive(&self, counter: u32) -> AnyResult<()> {
        let payload = counter.to_be_bytes();
        self.socket.send_to(&payload, self.address).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtp_state_randomize() {
        let state1 = RtpState::randomize();
        let state2 = RtpState::randomize();

        // Random states should be different (very high probability)
        assert!(
            state1.sequence != state2.sequence
                || state1.timestamp != state2.timestamp
                || state1.nonce != state2.nonce
        );
    }

    #[test]
    fn test_rtp_state_next() {
        let mut state = RtpState {
            sequence: 100,
            timestamp: 1000,
            nonce: 50,
        };

        let (seq, ts, n) = state.next();
        assert_eq!(seq, 100);
        assert_eq!(ts, 1000);
        assert_eq!(n, 50);

        // State should be incremented
        assert_eq!(state.sequence, 101);
        assert_eq!(state.timestamp, 1000 + RTP_TIMESTAMP_STEP);
        assert_eq!(state.nonce, 51);
    }

    #[test]
    fn test_rtp_state_next_wrapping() {
        let mut state = RtpState {
            sequence: u16::MAX,
            timestamp: u32::MAX,
            nonce: u32::MAX,
        };

        let (seq, ts, n) = state.next();
        assert_eq!(seq, u16::MAX);
        assert_eq!(ts, u32::MAX);
        assert_eq!(n, u32::MAX);

        // Should wrap around
        assert_eq!(state.sequence, 0);
        assert_eq!(state.timestamp, u32::MAX.wrapping_add(RTP_TIMESTAMP_STEP));
        assert_eq!(state.nonce, 0);
    }

    #[test]
    fn test_rtp_state_multiple_calls() {
        let mut state = RtpState {
            sequence: 0,
            timestamp: 0,
            nonce: 0,
        };

        for i in 0..10 {
            let (seq, ts, n) = state.next();
            assert_eq!(seq, i as u16);
            assert_eq!(ts, i * RTP_TIMESTAMP_STEP);
            assert_eq!(n, i as u32);
        }

        assert_eq!(state.sequence, 10);
        assert_eq!(state.timestamp, 10 * RTP_TIMESTAMP_STEP);
        assert_eq!(state.nonce, 10);
    }

    #[test]
    fn test_rtp_state_serialization() {
        let state = RtpState {
            sequence: 12345,
            timestamp: 987654321,
            nonce: 555,
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: RtpState = serde_json::from_str(&json).unwrap();

        assert_eq!(state.sequence, deserialized.sequence);
        assert_eq!(state.timestamp, deserialized.timestamp);
        assert_eq!(state.nonce, deserialized.nonce);
    }

    #[test]
    fn test_rtp_state_clone() {
        let state = RtpState {
            sequence: 100,
            timestamp: 2000,
            nonce: 300,
        };

        let cloned = state.clone();
        assert_eq!(state.sequence, cloned.sequence);
        assert_eq!(state.timestamp, cloned.timestamp);
        assert_eq!(state.nonce, cloned.nonce);
    }

    #[test]
    fn test_rtp_state_copy() {
        let state = RtpState {
            sequence: 100,
            timestamp: 2000,
            nonce: 300,
        };

        let copied = state;
        assert_eq!(state.sequence, copied.sequence);
        assert_eq!(state.timestamp, copied.timestamp);
        assert_eq!(state.nonce, copied.nonce);
    }

    #[test]
    fn test_rtp_state_debug() {
        let state = RtpState {
            sequence: 100,
            timestamp: 2000,
            nonce: 300,
        };

        let debug_str = format!("{:?}", state);
        assert!(debug_str.contains("100"));
        assert!(debug_str.contains("2000"));
        assert!(debug_str.contains("300"));
    }
}
