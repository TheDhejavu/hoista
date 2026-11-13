//! Which events a webhook gets, and which addresses Hoista will send to.

use crate::domain::model::Webhook;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether `webhook` wants an event of `kind` for project `project_id`.
#[must_use]
pub fn wants(webhook: &Webhook, kind: &str, project_id: Option<&str>) -> bool {
    if webhook
        .project_id
        .as_deref()
        .is_some_and(|only| Some(only) != project_id)
    {
        return false;
    }
    webhook.events.is_empty()
        || webhook
            .events
            .iter()
            .any(|wanted| match wanted.strip_suffix(".*") {
                Some(family) => kind.starts_with(family) && kind[family.len()..].starts_with('.'),
                None => wanted == kind,
            })
}

/// A public IPv4 address: not this computer, a private network, link-local
/// (cloud metadata lives there), shared (carrier-grade NAT), documentation,
/// multicast or reserved.
#[must_use]
pub fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0)
        || (a == 198 && (18..20).contains(&b)))
}

/// A public IPv6 address, with IPv4-mapped ones checked as IPv4.
#[must_use]
pub fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00 // unique local
        || (first & 0xffc0) == 0xfe80 // link-local
        || first == 0x2001 && ip.segments()[1] == 0x0db8 // documentation
        || first == 0x0064 && ip.segments()[1] == 0xff9b) // NAT64, can reach private IPv4
}

/// A public address of either kind.
#[must_use]
pub fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

#[cfg(test)]
mod tests {
    use crate::webhooks::rules::*;

    fn hook(events: &[&str], project: Option<&str>) -> Webhook {
        Webhook {
            id: "w".into(),
            url: "https://agent.example/hook".into(),
            events: events.iter().map(|kind| (*kind).to_owned()).collect(),
            project_id: project.map(str::to_owned),
            created_at: String::new(),
        }
    }

    #[test]
    fn events_match_by_kind_family_and_project() {
        assert!(wants(&hook(&[], None), "deploy.failed", None));
        assert!(wants(
            &hook(&["deploy.*"], None),
            "deploy.failed",
            Some("shop")
        ));
        assert!(!wants(
            &hook(&["deploy.*"], None),
            "deployment.failed",
            None
        ));
        assert!(!wants(&hook(&["deploy.*"], None), "deploy", None));
        assert!(wants(
            &hook(&["approval.created"], None),
            "approval.created",
            None
        ));
        assert!(!wants(
            &hook(&["approval.created"], None),
            "approval.resolved",
            None
        ));
        assert!(!wants(
            &hook(&[], Some("shop")),
            "deploy.failed",
            Some("blog")
        ));
        assert!(!wants(&hook(&[], Some("shop")), "account.expired", None));
    }

    #[test]
    fn private_and_metadata_addresses_are_refused() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::a00:1",
        ] {
            assert!(!public(private.parse().expect("ip")), "{private}");
        }
        for open in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public(open.parse().expect("ip")), "{open}");
        }
    }
}
