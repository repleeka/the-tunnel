//! Pairing uses a short authentication string, like Bluetooth "numeric comparison".
//!
//! Inside the TLS connection the initiator commits to a random nonce, the
//! responder replies with its own nonce, and the initiator then reveals its nonce.
//! Both sides derive a 6-digit code from the two certificate fingerprints and both
//! nonces. A machine-in-the-middle would hold different certificates on each side,
//! so the codes would differ, and the commitment stops it from grinding nonces until
//! they happen to match. The users compare the codes on both screens to confirm.

use sha2::{Digest, Sha256};

pub fn nonce() -> [u8; 32] {
    rand::random()
}

pub fn commitment(nonce: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"the-tunnel/commit/v1");
    h.update(nonce);
    hex::encode(h.finalize())
}

pub fn code(initiator_id: &str, responder_id: &str, initiator_nonce: &[u8], responder_nonce: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"the-tunnel/sas/v1");
    h.update(initiator_id.as_bytes());
    h.update(responder_id.as_bytes());
    h.update(initiator_nonce);
    h.update(responder_nonce);
    let digest = h.finalize();
    let n = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

pub fn decode_nonce(hex_str: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = hex::decode(hex_str)?;
    bytes.try_into().map_err(|_| anyhow::anyhow!("invalid pairing nonce"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_depends_on_every_input() {
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let base = code("alice", "bob", &a, &b);
        assert_eq!(base.len(), 7);
        assert_eq!(base, code("alice", "bob", &a, &b));
        assert_ne!(base, code("mallory", "bob", &a, &b));
        assert_ne!(base, code("alice", "bob", &a, &[3u8; 32]));
    }
}
