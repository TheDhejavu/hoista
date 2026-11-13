//! Custom domains: which project and resource a domain points at, and
//! whether the provider says it's ready. Not knowing never counts as ready.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Whether the provider says something is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DomainReadiness {
    /// Ready.
    Active,
    /// Waiting for DNS, or for the certificate to be issued.
    Pending,
    /// Failed, or switched off.
    Failing,
    /// The provider doesn't say, or couldn't be asked.
    Unknown,
}

/// A DNS record the provider suggests. Some are alternatives to each other:
/// one routing record is enough, not all of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct DomainDnsRecord {
    /// The record's full name.
    pub name: String,
    /// Its type.
    pub record_type: DomainDnsRecordType,
    /// Its value: an address, a CNAME target or TXT text, without quotes.
    pub value: String,
    /// What it's for.
    pub purpose: DomainDnsPurpose,
}

/// The DNS record types a provider may ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum DomainDnsRecordType {
    /// IPv4 address.
    A,
    /// IPv6 address.
    AAAA,
    /// Another name to follow.
    CNAME,
    /// Text, usually to prove the domain is yours.
    TXT,
}

/// What a suggested DNS record is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DomainDnsPurpose {
    /// Sends visitors to the resource. A CNAME or address records, not both.
    Routing,
    /// Lets the provider issue the certificate (an ACME challenge).
    AcmeChallenge,
    /// Proves the domain is yours. Any one of these is enough.
    Ownership,
}

/// What the provider said about a domain, when it was added or last checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct DomainObservation {
    /// The DNS zone's id at the provider. Empty when the provider doesn't run the domain's DNS.
    pub zone_id: String,
    /// The DNS zone's name: the provider's, or the one the person chose.
    pub zone_name: String,
    /// The nameservers the domain has to use, as the provider says.
    pub nameservers: Vec<String>,
    /// Where the domain is registered, when the provider says. It can be out of date.
    pub registrar: Option<String>,
    /// Whether DNS is set up, by the provider's own check.
    pub dns: DomainReadiness,
    /// Whether the HTTPS certificate is ready. DNS can be ready before it is.
    pub https: DomainReadiness,
    /// The provider's id for the domain.
    pub provider_domain_id: Option<String>,
    /// The provider's id for its certificate, when known.
    pub certificate_id: Option<String>,
    /// DNS records the provider suggests. Hoista shows them; it never adds them itself.
    pub dns_records: Vec<DomainDnsRecord>,
    /// Anything the person should know, such as a step only they can do.
    pub warnings: Vec<String>,
}

/// A custom domain and the resource it points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct CustomDomain {
    /// Its id in Hoista.
    pub id: String,
    /// The project.
    pub project_id: String,
    /// The resource that answers on it.
    pub resource_id: String,
    /// The domain, in lowercase ASCII, without `https://` or a path.
    pub hostname: String,
    /// What the provider last said about it.
    pub observed: DomainObservation,
    /// When that was (RFC 3339, UTC).
    pub checked_at: String,
}

/// The longest a domain name can be.
pub const MAX_HOSTNAME_CHARS: usize = 253;
/// The longest one label of a domain name (between dots) can be.
pub const MAX_LABEL_CHARS: usize = 63;

/// Whether `name` is a domain name: not a URL, an IP address or a wildcard.
pub fn valid_hostname(name: &str) -> bool {
    name.len() <= MAX_HOSTNAME_CHARS
        && name.contains('.')
        && name.parse::<std::net::IpAddr>().is_err()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= MAX_LABEL_CHARS
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Whether `hostname` is `zone` or a subdomain of it. `evilexample.com` isn't
/// in `example.com`.
pub fn belongs_to_zone(hostname: &str, zone: &str) -> bool {
    valid_hostname(hostname)
        && valid_hostname(zone)
        && (hostname == zone
            || hostname
                .strip_suffix(zone)
                .is_some_and(|prefix| prefix.ends_with('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_reject_urls_ips_and_zone_suffix_lookalikes() {
        assert!(belongs_to_zone("app.example.co.uk", "example.co.uk"));
        assert!(belongs_to_zone("example.com", "example.com"));
        assert!(!belongs_to_zone("evilexample.com", "example.com"));
        for host in [
            "https://example.com",
            "127.0.0.1",
            "example.com/path",
            "a..com",
            "*.example.com",
            "-a.example.com",
            "example.com.",
            "example.com:443",
        ] {
            assert!(!valid_hostname(host));
        }
    }
}
