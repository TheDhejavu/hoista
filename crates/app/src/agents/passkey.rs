//! Passkeys for a Hoista server's owner (WebAuthn): ES256 and Ed25519 keys, no
//! attestation, user verification required.
//!
//! The browser sends the SPKI public key and raw authenticator data, so no CBOR
//! is parsed. Every check is on bytes the authenticator signed or Hoista issued.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// COSE algorithm ids.
const ES256: i32 = -7;
const ED25519: i32 = -8;
/// SPKI prefixes before the raw key.
const P256_SPKI: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];
const ED25519_SPKI: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
/// Raw key lengths after those prefixes.
const P256_KEY_BYTES: usize = 65;
const ED25519_KEY_BYTES: usize = 32;
/// Authenticator data flags.
const USER_PRESENT: u8 = 0x01;
const USER_VERIFIED: u8 = 0x04;
const ATTESTED: u8 = 0x40;
// Authenticator data layout: the relying party's SHA-256, a flags byte, a
// four-byte counter, then (when attested) a 16-byte AAGUID, the credential
// id's two-byte length, and the id.
const RP_HASH: std::ops::Range<usize> = 0..32;
const FLAGS: usize = 32;
const COUNTER: std::ops::Range<usize> = 33..37;
const ID_LENGTH: std::ops::Range<usize> = 53..55;
const ID_START: usize = ID_LENGTH.end;

const CHALLENGE_BYTES: usize = 32;
/// A claim code: 12 characters, in groups of four.
const CLAIM_CODE_CHARS: usize = 12;
const CLAIM_CODE_GROUP: usize = 4;
const MAX_NAME_CHARS: usize = 60;

/// A registered owner passkey. Holds only public material.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Passkey {
    /// Credential id, base64url.
    pub id: String,
    /// Raw public key: a 65-byte P-256 point or a 32-byte Ed25519 key.
    pub public_key: Vec<u8>,
    /// COSE algorithm (-7 or -8).
    pub alg: i32,
    /// Last signature counter seen.
    pub sign_count: u32,
    /// A name the person gave it, such as the device.
    pub name: String,
}

/// What the browser sends after `navigator.credentials.create()`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    /// `rawId`, base64url.
    pub id: String,
    /// `response.clientDataJSON`, base64url.
    pub client_data: String,
    /// `response.getAuthenticatorData()`, base64url.
    pub authenticator_data: String,
    /// `response.getPublicKey()` (SPKI DER), base64url.
    pub public_key: String,
    /// `response.getPublicKeyAlgorithm()`.
    pub alg: i32,
    /// The device name the person typed, if any.
    pub name: Option<String>,
}

/// What the browser sends after `navigator.credentials.get()`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    /// `rawId`, base64url.
    pub id: String,
    /// `response.clientDataJSON`, base64url.
    pub client_data: String,
    /// `response.authenticatorData`, base64url.
    pub authenticator_data: String,
    /// `response.signature`, base64url.
    pub signature: String,
}

/// Where a ceremony must have happened.
pub struct Expected<'a> {
    /// The challenge Hoista issued, base64url.
    pub challenge: &'a str,
    /// `https://host` of this Hoista.
    pub origin: &'a str,
    /// The host name (relying party id).
    pub rp_id: &'a str,
}

/// A fresh 32-byte challenge, base64url.
#[must_use]
pub fn challenge() -> String {
    use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
    let mut bytes = [0u8; CHALLENGE_BYTES];
    OsRng.fill_bytes(&mut bytes);
    b64(&bytes)
}

/// A one-time code that lets the first owner claim a new server,
/// `ABCD-EFGH-JKLM`, from letters and digits that can't be misread.
#[must_use]
pub fn claim_code() -> String {
    use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut code = String::with_capacity(CLAIM_CODE_CHARS + CLAIM_CODE_CHARS / CLAIM_CODE_GROUP);
    let mut index = 0;
    while index < CLAIM_CODE_CHARS {
        let mut byte = [0u8; 1];
        OsRng.fill_bytes(&mut byte);
        // Skip values that would favour some letters.
        if usize::from(byte[0]) >= 256 - 256 % ALPHABET.len() {
            continue;
        }
        if index > 0 && index % CLAIM_CODE_GROUP == 0 {
            code.push('-');
        }
        code.push(char::from(ALPHABET[usize::from(byte[0]) % ALPHABET.len()]));
        index += 1;
    }
    code
}

/// Whether `code` looks like one [`claim_code`] made.
#[must_use]
pub fn is_claim_code(code: &str) -> bool {
    code.len() == 14
        && code.split('-').count() == 3
        && code.split('-').all(|group| {
            group.len() == 4
                && group
                    .bytes()
                    .all(|b| b"ABCDEFGHJKMNPQRSTUVWXYZ23456789".contains(&b))
        })
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Result<Vec<u8>, &'static str> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .map_err(|_| "The passkey answer wasn't readable.")
}

fn same(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: bool,
}

fn check_client_data(raw: &[u8], kind: &str, expected: &Expected) -> Result<(), &'static str> {
    let data: ClientData =
        serde_json::from_slice(raw).map_err(|_| "The passkey answer wasn't readable.")?;
    if data.kind != kind {
        return Err("That passkey answer was for something else.");
    }
    if !same(data.challenge.as_bytes(), expected.challenge.as_bytes()) {
        return Err("That passkey answer is out of date. Try again.");
    }
    if data.origin != expected.origin || data.cross_origin {
        return Err("That passkey answer came from another site.");
    }
    Ok(())
}

/// Checks the relying party, presence and verification; returns the counter.
fn check_authenticator(data: &[u8], expected: &Expected) -> Result<u32, &'static str> {
    if data.len() < COUNTER.end {
        return Err("The passkey answer wasn't readable.");
    }
    if !same(&data[RP_HASH], &Sha256::digest(expected.rp_id.as_bytes())) {
        return Err("That passkey belongs to another site.");
    }
    if data[FLAGS] & USER_PRESENT == 0 || data[FLAGS] & USER_VERIFIED == 0 {
        return Err("The passkey didn't confirm it was you (no fingerprint, face or PIN).");
    }
    let counter: [u8; 4] = data[COUNTER]
        .try_into()
        .map_err(|_| "The passkey answer wasn't readable.")?;
    Ok(u32::from_be_bytes(counter))
}

/// Verifies a new passkey made in answer to `expected.challenge`.
///
/// # Errors
/// Explains in plain words what didn't match.
pub fn register(answer: &Registration, expected: &Expected) -> Result<Passkey, &'static str> {
    check_client_data(&unb64(&answer.client_data)?, "webauthn.create", expected)?;
    let data = unb64(&answer.authenticator_data)?;
    let sign_count = check_authenticator(&data, expected)?;
    if data[FLAGS] & ATTESTED == 0 || data.len() < ID_START {
        return Err("The passkey answer wasn't readable.");
    }
    let length = usize::from(u16::from_be_bytes([
        data[ID_LENGTH.start],
        data[ID_LENGTH.start + 1],
    ]));
    let id = data
        .get(ID_START..ID_START + length)
        .ok_or("The passkey answer wasn't readable.")?;
    if !same(id, &unb64(&answer.id)?) {
        return Err("The passkey answer wasn't readable.");
    }
    let spki = unb64(&answer.public_key)?;
    let public_key = match answer.alg {
        ES256 if spki.len() == P256_SPKI.len() + P256_KEY_BYTES && spki.starts_with(&P256_SPKI) => {
            spki[P256_SPKI.len()..].to_vec()
        }
        ED25519
            if spki.len() == ED25519_SPKI.len() + ED25519_KEY_BYTES
                && spki.starts_with(&ED25519_SPKI) =>
        {
            spki[ED25519_SPKI.len()..].to_vec()
        }
        _ => return Err("This passkey uses a kind of key Hoista doesn't accept yet."),
    };
    let name = answer
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Passkey")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    Ok(Passkey {
        id: b64(id),
        public_key,
        alg: answer.alg,
        sign_count,
        name,
    })
}

/// Verifies a sign-in with `key`, updating its counter.
///
/// # Errors
/// Explains in plain words what didn't match.
pub fn verify(
    answer: &Assertion,
    key: &mut Passkey,
    expected: &Expected,
) -> Result<(), &'static str> {
    if !same(&unb64(&answer.id)?, &unb64(&key.id)?) {
        return Err("That isn't one of this Hoista's passkeys.");
    }
    let client = unb64(&answer.client_data)?;
    check_client_data(&client, "webauthn.get", expected)?;
    let data = unb64(&answer.authenticator_data)?;
    let count = check_authenticator(&data, expected)?;
    let mut signed = data.clone();
    signed.extend_from_slice(&Sha256::digest(&client));
    let algorithm: &dyn ring::signature::VerificationAlgorithm = match key.alg {
        ES256 => &ring::signature::ECDSA_P256_SHA256_ASN1,
        ED25519 => &ring::signature::ED25519,
        _ => return Err("This passkey uses a kind of key Hoista doesn't accept yet."),
    };
    ring::signature::UnparsedPublicKey::new(algorithm, &key.public_key)
        .verify(&signed, &unb64(&answer.signature)?)
        .map_err(|_| "The passkey's signature didn't check out.")?;
    // A counter that doesn't move forward can mean a copied key.
    if (count != 0 || key.sign_count != 0) && count <= key.sign_count {
        return Err("This passkey's counter went backwards, so Hoista refused it.");
    }
    key.sign_count = count;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::agents::passkey::*;
    use ring::{
        rand::SystemRandom,
        signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
    };

    const ORIGIN: &str = "https://hoista.example.com";
    const RP: &str = "hoista.example.com";

    fn client(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
        format!(r#"{{"type":"{kind}","challenge":"{challenge}","origin":"{origin}","crossOrigin":false}}"#).into_bytes()
    }

    fn auth(flags: u8, count: u32, attested: Option<&[u8]>) -> Vec<u8> {
        let mut data = Sha256::digest(RP.as_bytes()).to_vec();
        data.push(flags);
        data.extend_from_slice(&count.to_be_bytes());
        if let Some(id) = attested {
            data.extend_from_slice(&[0u8; 16]);
            data.extend_from_slice(&u16::try_from(id.len()).unwrap().to_be_bytes());
            data.extend_from_slice(id);
            data.extend_from_slice(&[0xa5]); // COSE key follows; not read
        }
        data
    }

    #[test]
    fn a_registered_passkey_signs_in_and_replays_are_refused() {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let mut spki = P256_SPKI.to_vec();
        spki.extend_from_slice(pair.public_key().as_ref());
        let id = b"credential-id-1";
        let made = challenge();
        let expected = Expected {
            challenge: &made,
            origin: ORIGIN,
            rp_id: RP,
        };
        let registration = Registration {
            id: b64(id),
            client_data: b64(&client("webauthn.create", &made, ORIGIN)),
            authenticator_data: b64(&auth(0x45, 0, Some(id))),
            public_key: b64(&spki),
            alg: ES256,
            name: Some("MacBook".into()),
        };
        let mut key = register(&registration, &expected).unwrap();
        assert_eq!(key.name, "MacBook");
        // Not verified (no UV flag), or another site: refused.
        let mut no_uv = Registration {
            authenticator_data: b64(&auth(0x41, 0, Some(id))),
            ..registration
        };
        assert!(register(&no_uv, &expected).is_err());
        no_uv.authenticator_data = b64(&auth(0x45, 0, Some(id)));
        no_uv.client_data = b64(&client("webauthn.create", &made, "https://evil.example"));
        assert!(register(&no_uv, &expected).is_err());

        let sign = |count: u32, challenge: &str, origin: &str| {
            let data = auth(0x05, count, None);
            let cdj = client("webauthn.get", challenge, origin);
            let mut signed = data.clone();
            signed.extend_from_slice(&Sha256::digest(&cdj));
            Assertion {
                id: b64(id),
                client_data: b64(&cdj),
                authenticator_data: b64(&data),
                signature: b64(pair.sign(&rng, &signed).unwrap().as_ref()),
            }
        };
        let now = challenge();
        let expected = Expected {
            challenge: &now,
            origin: ORIGIN,
            rp_id: RP,
        };
        verify(&sign(1, &now, ORIGIN), &mut key, &expected).unwrap();
        assert_eq!(key.sign_count, 1);
        // The same counter again, an old challenge, another origin: refused.
        assert!(verify(&sign(1, &now, ORIGIN), &mut key, &expected).is_err());
        assert!(verify(&sign(2, &made, ORIGIN), &mut key, &expected).is_err());
        assert!(verify(&sign(3, &now, "https://evil.example"), &mut key, &expected).is_err());
        // A forged signature: refused.
        let mut forged = sign(4, &now, ORIGIN);
        forged.signature = b64(&[0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01]);
        assert!(verify(&forged, &mut key, &expected).is_err());
        verify(&sign(5, &now, ORIGIN), &mut key, &expected).unwrap();
    }
}
