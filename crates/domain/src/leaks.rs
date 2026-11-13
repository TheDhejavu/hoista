//! Finds secrets in what's about to be deployed or shown: in build output,
//! in variables a browser will see, and in error text from tools. A finding
//! says where and what kind, never the value.

use crate::Secret;
use base64::Engine as _;
use regex::Regex;

/// Patterns of well-known secret formats, by the name a finding reports.
const PATTERNS: [(&str, &str); 9] = [
    ("stripe_secret", r"(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}"),
    (
        "openai_secret",
        r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}",
    ),
    ("aws_access_key", r"(?:AKIA|ASIA)[A-Z0-9]{16}"),
    (
        "github_token",
        r"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})",
    ),
    (
        "private_key",
        r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----",
    ),
    (
        "database_credentials",
        r"(?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?)://[^\s:/]+:[^\s@]+@",
    ),
    ("supabase_secret", r"sb_secret_[A-Za-z0-9_-]{16,}"),
    ("npm_token", r"npm_[A-Za-z0-9]{20,}"),
    (
        "registry_credential",
        r#"(?im)(?:_authToken|_auth|npmAuthToken|npmAuthIdent)\s*[:=]\s*["']?[^$\s"']{8,}"#,
    ),
];

/// Variable names with these prefixes are built into what browsers download.
pub const PUBLIC_PREFIXES: [&str; 3] = ["VITE_", "NEXT_PUBLIC_", "PUBLIC_"];

/// Words in a variable's name that say it's for the server only.
const SERVER_ONLY_WORDS: [&str; 5] = [
    "SECRET",
    "PRIVATE_KEY",
    "SERVICE_ROLE",
    "DATABASE_URL",
    "PASSWORD",
];

/// Longer than any real JWT; anything longer isn't looked at.
const MAX_JWT_BYTES: usize = 8192;

/// Where a secret was found and what kind it looks like. Safe to show and to log.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
    PartialEq,
    Eq,
)]
pub struct Leak {
    /// The file or variable it's in.
    pub location: String,
    /// What kind of secret, such as `github_token`.
    pub kind: String,
}

/// Looks for well-known secret formats, and for the exact secret values it's given.
pub struct LeakScanner {
    patterns: Vec<(&'static str, Regex)>,
}

impl LeakScanner {
    /// Compiles the patterns.
    ///
    /// # Errors
    /// Returns an error if one of [`PATTERNS`] isn't a valid regular expression.
    pub fn new() -> Result<Self, regex::Error> {
        let patterns = PATTERNS
            .into_iter()
            .map(|(name, pattern)| Regex::new(pattern).map(|pattern| (name, pattern)))
            .collect::<Result<_, _>>()?;
        Ok(Self { patterns })
    }

    /// What secrets `bytes` holds: any of `secrets` exactly, any well-known
    /// format, or a Supabase service-role key.
    pub fn scan(&self, location: &str, bytes: &[u8], secrets: &[Secret]) -> Vec<Leak> {
        let mut findings = Vec::new();
        for secret in secrets {
            let value = secret.expose().as_bytes();
            if !value.is_empty() && bytes.windows(value.len()).any(|window| window == value) {
                findings.push(Leak {
                    location: location.into(),
                    kind: "known_secret_value".into(),
                });
                break;
            }
        }
        let content = String::from_utf8_lossy(bytes);
        for (name, pattern) in &self.patterns {
            if pattern.is_match(&content) {
                findings.push(Leak {
                    location: location.into(),
                    kind: (*name).into(),
                });
            }
        }
        for candidate in content
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        {
            if is_service_role(candidate) {
                findings.push(Leak {
                    location: location.into(),
                    kind: "supabase_service_role".into(),
                });
                break;
            }
        }
        findings
    }

    /// What's wrong with putting `value` in the public variable `name`: a
    /// secret in it, or a name that says it's for the server only. Nothing
    /// for a variable browsers don't see.
    pub fn public_env(&self, name: &str, value: &Secret) -> Vec<Leak> {
        if !PUBLIC_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            return Vec::new();
        }
        let mut findings = self.scan(name, value.expose().as_bytes(), &[]);
        let upper = name.to_ascii_uppercase();
        if !value.expose().is_empty() && SERVER_ONLY_WORDS.iter().any(|word| upper.contains(word)) {
            findings.push(Leak {
                location: name.into(),
                kind: "public_server_secret".into(),
            });
        }
        findings
    }

    /// `text` with every secret it finds replaced by `[REDACTED]`. For error
    /// text from tools and providers, before it's shown or logged.
    pub fn redact(&self, text: &str, secrets: &[Secret]) -> String {
        let mut redacted = text.to_owned();
        for secret in secrets {
            if !secret.expose().is_empty() {
                redacted = redacted.replace(secret.expose(), "[REDACTED]");
            }
        }
        for (_, pattern) in &self.patterns {
            redacted = pattern.replace_all(&redacted, "[REDACTED]").into_owned();
        }
        let candidates = redacted
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
            .filter(|value| is_service_role(value))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for candidate in candidates {
            redacted = redacted.replace(&candidate, "[REDACTED]");
        }
        redacted
    }
}

/// Whether `value` is a JWT with the `service_role` role: a Supabase key
/// that gets past every access rule.
fn is_service_role(value: &str) -> bool {
    if value.len() > MAX_JWT_BYTES {
        return false;
    }
    // A JWT is header.payload.signature.
    let parts: Vec<&str> = value.split('.').collect();
    let [_, payload, _] = parts[..] else {
        return false;
    };
    let Ok(payload) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload) else {
        return false;
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&payload) else {
        return false;
    };
    json.get("role").and_then(serde_json::Value::as_str) == Some("service_role")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_patterns_do_not_match_inside_framework_identifiers() {
        let scanner = LeakScanner::new().unwrap();
        let path = "next/dist/server/app-render/after-task-async-storage-instance.js";
        assert!(scanner.scan("worker.js", path.as_bytes(), &[]).is_empty());
        assert_eq!(scanner.redact(path, &[]), path);
        for prefix in ["sk-", "sk-proj-", "sk-svcacct-"] {
            let value = format!("{prefix}{}", "a".repeat(48));
            let content = format!("key='{value}'");
            assert!(
                scanner
                    .scan("worker.js", content.as_bytes(), &[])
                    .iter()
                    .any(|finding| finding.kind == "openai_secret")
            );
            assert!(!scanner.redact(&content, &[]).contains(&value));
        }
    }

    #[test]
    fn detects_binary_embeds_keys_and_privileged_jwts_without_echoing_values() {
        let scanner = LeakScanner::new().unwrap();
        let secret = Secret::new("secret-not-a-pattern".into());
        let found = scanner.scan("bundle.js", b"\x00secret-not-a-pattern\xff", &[secret]);
        assert_eq!(found.len(), 1);
        assert!(!format!("{found:?}").contains("secret-not-a-pattern"));
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"role":"service_role"}"#);
        let token = Secret::new(format!("header.{payload}.signature"));
        assert!(!scanner.public_env("VITE_SUPABASE_KEY", &token).is_empty());
        let public = Secret::new("ordinary-public-id".into());
        assert!(scanner.public_env("VITE_PROJECT", &public).is_empty());
        assert!(
            !scanner
                .scan("diff", b"+-----BEGIN PRIVATE KEY-----", &[])
                .is_empty()
        );
    }
}
