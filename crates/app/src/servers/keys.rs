//! The two keys behind a server Hoista makes: the one Hoista signs in with,
//! and the server's own host key, made here so Hoista knows the server it
//! reaches is the one it made from the very first connection.
//!
//! Both are Ed25519, written in OpenSSH's formats so the server, `ssh` and
//! Hoista's SSH client all read them.

use crate::{domain::Secret, ports::catalog::EngineError};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair},
};

const KEY_TYPE: &str = "ssh-ed25519";
const SEED_BYTES: usize = 32;
/// OpenSSH wraps the base64 of a private key at 70 characters.
const PEM_LINE_CHARS: usize = 70;
/// An unencrypted OpenSSH private key pads its private part to this.
const BLOCK_BYTES: usize = 8;

/// One Ed25519 key.
pub struct ServerKey {
    seed: Secret,
    public: [u8; 32],
    comment: String,
}

impl ServerKey {
    /// A new random key.
    ///
    /// # Errors
    /// Reports a system without a random source.
    pub fn generate(comment: &str) -> Result<Self, EngineError> {
        let mut seed = zeroize::Zeroizing::new([0_u8; SEED_BYTES]);
        SystemRandom::new()
            .fill(seed.as_mut())
            .map_err(|_| EngineError::Store)?;
        Self::from_seed(&seed, comment)
    }

    fn from_seed(seed: &[u8; SEED_BYTES], comment: &str) -> Result<Self, EngineError> {
        let pair = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| EngineError::Store)?;
        let mut public = [0_u8; 32];
        public.copy_from_slice(pair.public_key().as_ref());
        Ok(Self {
            seed: Secret::new(STANDARD.encode(seed)),
            public,
            comment: comment.to_owned(),
        })
    }

    /// The public key as one `authorized_keys` line: `ssh-ed25519 AAAA… comment`.
    #[must_use]
    pub fn public_line(&self) -> String {
        format!(
            "{KEY_TYPE} {} {}",
            STANDARD.encode(self.public_blob()),
            self.comment
        )
    }

    fn public_blob(&self) -> Vec<u8> {
        let mut blob = Vec::new();
        string(&mut blob, KEY_TYPE.as_bytes());
        string(&mut blob, &self.public);
        blob
    }

    /// The private key in OpenSSH's own format, unencrypted: what goes in
    /// `~/.ssh/id_ed25519`, or a server's `/etc/ssh/ssh_host_ed25519_key`.
    ///
    /// # Errors
    /// Reports a system without a random source.
    pub fn private_openssh(&self) -> Result<Secret, EngineError> {
        let seed = STANDARD
            .decode(self.seed.expose())
            .map_err(|_| EngineError::Store)?;
        let seed = zeroize::Zeroizing::new(seed);
        let mut check = [0_u8; 4];
        SystemRandom::new()
            .fill(&mut check)
            .map_err(|_| EngineError::Store)?;
        let mut private = zeroize::Zeroizing::new(Vec::new());
        private.extend_from_slice(&check);
        private.extend_from_slice(&check);
        string(&mut private, KEY_TYPE.as_bytes());
        string(&mut private, &self.public);
        let mut both = zeroize::Zeroizing::new(Vec::with_capacity(64));
        both.extend_from_slice(&seed);
        both.extend_from_slice(&self.public);
        string(&mut private, &both);
        string(&mut private, self.comment.as_bytes());
        let mut pad = 1_u8;
        while private.len() % BLOCK_BYTES != 0 {
            private.push(pad);
            pad += 1;
        }
        let mut whole = zeroize::Zeroizing::new(b"openssh-key-v1\0".to_vec());
        string(&mut whole, b"none");
        string(&mut whole, b"none");
        string(&mut whole, b"");
        whole.extend_from_slice(&1_u32.to_be_bytes());
        string(&mut whole, &self.public_blob());
        string(&mut whole, &private);
        let encoded = zeroize::Zeroizing::new(STANDARD.encode(whole.as_slice()));
        let mut text = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
        for line in encoded.as_bytes().chunks(PEM_LINE_CHARS) {
            text.push_str(std::str::from_utf8(line).map_err(|_| EngineError::Store)?);
            text.push('\n');
        }
        text.push_str("-----END OPENSSH PRIVATE KEY-----\n");
        Ok(Secret::new(text))
    }
}

/// An SSH `string`: its length as four big-endian bytes, then the bytes.
fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032's first test key, whose public key is known.
    const SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    #[test]
    fn the_public_line_is_what_openssh_writes() {
        let key = ServerKey::from_seed(&SEED, "hoista").unwrap();
        assert_eq!(
            key.public_line(),
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINdamAGCsQq31Uv+08lkBzoO4XLz2qYjJa8CGmj3B1Ea hoista"
        );
    }

    #[test]
    fn the_private_key_is_in_openssh_format() {
        let key = ServerKey::from_seed(&SEED, "hoista").unwrap();
        let private = key.private_openssh().unwrap();
        let text = private.expose();
        assert!(text.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
        assert!(text.ends_with("-----END OPENSSH PRIVATE KEY-----\n"));
        assert!(text.lines().all(|line| line.len() <= PEM_LINE_CHARS));
        let body: String = text
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let bytes = STANDARD.decode(body).unwrap();
        assert!(bytes.starts_with(b"openssh-key-v1\0"));
        // The seed and the public key are both in the private part.
        let pair = [&SEED[..], &key.public[..]].concat();
        assert!(bytes.windows(64).any(|window| window == pair.as_slice()));
    }

    #[test]
    fn new_keys_differ() {
        let one = ServerKey::generate("a").unwrap();
        let two = ServerKey::generate("a").unwrap();
        assert_ne!(one.public_line(), two.public_line());
    }
}
