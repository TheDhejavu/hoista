//! Making a server: the key Hoista signs in with, a firewall, and the
//! server itself, set up on first boot by cloud-init.
//!
//! Hetzner keeps a server's `user_data`, and anyone with a Read token can
//! fetch it, so it holds nothing that stays secret: the host key in it lets
//! Hoista check the server on its first connection, and Hoista then replaces
//! that key on the server (see `hoista_app::servers`).

use super::{Hetzner, invalid, text};
use crate::{
    application::{
        ports::{catalog::EngineError, providers::ProviderNew},
        servers::{SERVER_USER, ServerSetup},
    },
    domain::Secret,
};
use reqwest::Method;
use serde_json::{Value, json};

/// The one image Hoista sets servers up on: Ubuntu's current long-term
/// release, so the setup has one target.
const IMAGE: &str = "ubuntu-24.04";
/// What marks everything Hoista makes for one server, so it goes with it.
const KEY_LABEL: &str = "hoista-server";
/// The label that puts a server in a Hoista project.
pub(super) const PROJECT_LABEL: &str = "hoista.project";
/// Version of the setup below, written to `/etc/hoista/version`.
pub(super) const SETUP_VERSION: u32 = 1;
/// Ports open to everyone: SSH, and HTTP and HTTPS for what the server runs.
const OPEN_PORTS: [&str; 3] = ["22", "80", "443"];

/// What to make: the server's settings and its keys.
pub(super) struct NewServer<'a> {
    name: &'a str,
    server_type: Option<&'a str>,
    location: Option<&'a str>,
    /// The Hoista project it's for, when Hetzner takes it as a label value.
    project: Option<&'a str>,
    /// A volume to attach and mount, in GB.
    volume_gb: Option<u32>,
    backups: bool,
    setup: &'a ServerSetup,
}

/// Hetzner's smallest and largest volumes, in GB.
const VOLUME_GB: std::ops::RangeInclusive<u32> = 10..=10240;

impl<'a> NewServer<'a> {
    /// The server in `what`, or None when it asks for something else.
    ///
    /// # Errors
    /// Reports a server without keys, or a name Hetzner won't take.
    pub(super) fn from(
        what: &ProviderNew<'a>,
        project: &'a str,
    ) -> Result<Option<Self>, EngineError> {
        let ProviderNew::Server {
            name,
            server_type,
            location,
            volume_gb,
            backups,
            setup,
        } = *what
        else {
            return Ok(None);
        };
        let setup = setup.ok_or(EngineError::Invalid(
            "A server needs the keys Hoista makes for it.",
        ))?;
        if volume_gb.is_some_and(|gb| !VOLUME_GB.contains(&gb)) {
            return Err(EngineError::Invalid("A Hetzner volume is 10 to 10240 GB."));
        }
        if !crate::domain::model::valid_slug(name) {
            return Err(EngineError::Invalid(
                "Use a lowercase name with letters, digits and hyphens.",
            ));
        }
        let plain = |value: &str| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        if [server_type, location]
            .into_iter()
            .flatten()
            .any(|value| !plain(value))
        {
            return Err(EngineError::Invalid(
                "Give the server type and location as Hetzner names them, such as cx23 and fsn1.",
            ));
        }
        // Hetzner's label values: up to 63 letters, digits, `-`, `_` and `.`,
        // starting and ending with a letter or digit.
        let project = Some(project).filter(|project| {
            plain(project)
                && project.len() <= 63
                && project.starts_with(|c: char| c.is_ascii_alphanumeric())
                && project.ends_with(|c: char| c.is_ascii_alphanumeric())
        });
        Ok(Some(Self {
            name,
            server_type,
            location,
            project,
            volume_gb,
            backups,
            setup,
        }))
    }

    /// What everything made for this server is labelled with: a short hash
    /// of its sign-in key, the same on every retry.
    fn mark(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.setup.authorized_key.as_bytes());
        digest[..4].iter().map(|b| format!("{b:02x}")).collect()
    }

    fn labels(&self) -> Value {
        json!({ "managed-by": "hoista", KEY_LABEL: self.mark() })
    }

    /// The server's own labels: the ones above, and its Hoista project.
    fn server_labels(&self) -> Value {
        let mut labels = self.labels();
        if let Some(project) = self.project {
            labels[PROJECT_LABEL] = json!(project);
        }
        labels
    }
}

impl Hetzner {
    /// Makes the server: its sign-in key and firewall first (or the ones an
    /// earlier try made), then the server, set up on first boot.
    pub(super) async fn make_server(
        &self,
        token: &Secret,
        new: &NewServer<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let mark = new.mark();
        let key_name = format!("hoista-{}-{mark}", new.name);
        let key = match self.named(token, "ssh_keys", &key_name).await? {
            Some(key) => key,
            None => self
                .call(
                    token,
                    Method::POST,
                    &["ssh_keys"],
                    &[],
                    Some(&json!({
                        "name": key_name,
                        "public_key": new.setup.authorized_key,
                        "labels": new.labels(),
                    })),
                )
                .await?["ssh_key"]
                .clone(),
        };
        let key_id = key["id"].as_u64().ok_or_else(invalid)?;
        let firewall = match self.named(token, "firewalls", &key_name).await? {
            Some(firewall) => firewall,
            None => self
                .call(
                    token,
                    Method::POST,
                    &["firewalls"],
                    &[],
                    Some(&json!({
                        "name": key_name,
                        "labels": new.labels(),
                        "rules": firewall_rules(),
                    })),
                )
                .await?["firewall"]
                .clone(),
        };
        let firewall_id = firewall["id"].as_u64().ok_or_else(invalid)?;
        let server_type = match new.server_type {
            Some(name) => name.to_owned(),
            None => self.smallest_type(token, new.location).await?,
        };
        let mut body = json!({
            "name": new.name,
            "server_type": server_type,
            "image": IMAGE,
            "ssh_keys": [key_id],
            "firewalls": [{ "firewall": firewall_id }],
            "user_data": cloud_init(new.name, new.setup),
            "labels": new.server_labels(),
            "start_after_create": true,
        });
        if let Some(location) = new.location {
            body["location"] = json!(location);
        }
        let made = self
            .call(token, Method::POST, &["servers"], &[], Some(&body))
            .await?;
        let server = &made["server"];
        let id = server["id"].as_u64().ok_or_else(invalid)?;
        let address = text(&server["public_net"]["ipv4"], "ip")
            .or_else(|| text(&server["public_net"]["ipv6"], "ip"));
        let place = super::location(server).unwrap_or_else(|| "Hetzner".to_owned());
        let mut made = vec![format!("server:{id}")];
        let mut extras = Vec::new();
        if let Some(gb) = new.volume_gb {
            let volume_name = format!("{}-data", new.name);
            let volume = match self.named(token, "volumes", &volume_name).await? {
                Some(volume) => volume,
                None => self
                    .call(
                        token,
                        Method::POST,
                        &["volumes"],
                        &[],
                        Some(&json!({
                            "name": volume_name,
                            "size": gb,
                            "server": id,
                            "automount": true,
                            "format": "ext4",
                            "labels": new.labels(),
                        })),
                    )
                    .await?["volume"]
                    .clone(),
            };
            let volume_id = volume["id"].as_u64().ok_or_else(invalid)?;
            made.push(format!("volume:{volume_id}"));
            extras.push(format!(
                "a {gb} GB volume, {volume_name}, mounted at /mnt/HC_Volume_{volume_id}"
            ));
        }
        if new.backups {
            self.call(
                token,
                Method::POST,
                &["servers", &id.to_string(), "actions", "enable_backup"],
                &[],
                None,
            )
            .await?;
            extras.push("Hetzner's backups on".to_owned());
        }
        let with = match extras.as_slice() {
            [] => String::new(),
            [one] => format!(", with {one}"),
            [first, second] => format!(", with {first} and {second}"),
            _ => format!(", with {}", extras.join(", ")),
        };
        let summary = format!(
            "Made server {} ({server_type}, {place}){}{with}. It sets itself up over the next few minutes; Hoista signs in as {SERVER_USER}",
            new.name,
            address.map_or_else(String::new, |address| format!(" at {address}"))
        );
        Ok((made, summary))
    }

    /// The item named `name` in one of Hetzner's lists, if there is one.
    async fn named(
        &self,
        token: &Secret,
        list: &str,
        name: &str,
    ) -> Result<Option<Value>, EngineError> {
        let found = self
            .call(token, Method::GET, &[list], &[("name", name)], None)
            .await?;
        Ok(found[list]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == name))
            .cloned())
    }

    /// The smallest current x86 server type, offered at `location` when one
    /// is given.
    async fn smallest_type(
        &self,
        token: &Secret,
        location: Option<&str>,
    ) -> Result<String, EngineError> {
        let types = self
            .call(
                token,
                Method::GET,
                &["server_types"],
                &[("per_page", "50")],
                None,
            )
            .await?;
        pick_smallest(&types, location).ok_or(EngineError::Invalid(
            "Hetzner offers no server type there. Pick another location.",
        ))
    }

    /// Removes the sign-in key and firewall made with a server that's gone.
    pub(super) async fn remove_made_with(
        &self,
        token: &Secret,
        server: &Value,
    ) -> Result<(), EngineError> {
        let Some(mark) = server["labels"][KEY_LABEL].as_str() else {
            return Ok(());
        };
        if mark.len() != 8 || !mark.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(());
        }
        let selector = format!("{KEY_LABEL}={mark}");
        for list in ["ssh_keys", "firewalls"] {
            let found = self
                .call(
                    token,
                    Method::GET,
                    &[list],
                    &[("label_selector", &selector)],
                    None,
                )
                .await?;
            for item in found[list].as_array().into_iter().flatten() {
                if let Some(id) = item["id"].as_u64() {
                    self.call(token, Method::DELETE, &[list, &id.to_string()], &[], None)
                        .await?;
                }
            }
        }
        Ok(())
    }
}

/// SSH, HTTP, HTTPS and ping in, from anywhere; everything out.
fn firewall_rules() -> Value {
    let anywhere = json!(["0.0.0.0/0", "::/0"]);
    let mut rules: Vec<Value> = OPEN_PORTS
        .iter()
        .map(|port| {
            json!({
                "direction": "in",
                "protocol": "tcp",
                "port": port,
                "source_ips": anywhere,
            })
        })
        .collect();
    rules.push(json!({ "direction": "in", "protocol": "icmp", "source_ips": anywhere }));
    Value::Array(rules)
}

fn pick_smallest(types: &Value, location: Option<&str>) -> Option<String> {
    types["server_types"]
        .as_array()?
        .iter()
        .filter(|kind| kind["architecture"] == "x86")
        .filter(|kind| kind["deprecated"] != true && kind["deprecation"].is_null())
        .filter(|kind| {
            location.is_none_or(|location| {
                kind["prices"]
                    .as_array()
                    .is_some_and(|prices| prices.iter().any(|price| price["location"] == location))
            })
        })
        .min_by(|a, b| {
            let size = |kind: &Value| {
                (
                    kind["memory"].as_f64().unwrap_or(f64::MAX),
                    kind["cores"].as_u64().unwrap_or(u64::MAX),
                )
            };
            size(a)
                .partial_cmp(&size(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .and_then(|kind| text(kind, "name"))
}

/// What the server runs on first boot: the `hoista` user with only Hoista's
/// key and passwordless sudo, the host key Hoista made, SSH without
/// passwords or root, Docker, automatic security updates, UTC. Docker comes
/// from Ubuntu's own packages, so automatic security updates cover it.
fn cloud_init(name: &str, setup: &ServerSetup) -> String {
    let indent = |text: &str| {
        text.lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"#cloud-config
hostname: {name}
timezone: UTC
ssh_pwauth: false
disable_root: true
ssh_deletekeys: true
ssh_genkeytypes: []
ssh_keys:
  ed25519_private: |
{host_private}
  ed25519_public: {host_public}
users:
  - name: {SERVER_USER}
    shell: /bin/bash
    lock_passwd: true
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - {authorized}
package_update: true
package_upgrade: true
packages:
  - ca-certificates
  - curl
  - docker.io
  - docker-compose-v2
  - unattended-upgrades
write_files:
  - path: /etc/ssh/sshd_config.d/10-hoista.conf
    permissions: "0644"
    content: |
      PasswordAuthentication no
      KbdInteractiveAuthentication no
      PermitRootLogin no
      AllowUsers {SERVER_USER}
  - path: /etc/hoista/version
    permissions: "0644"
    content: "{SETUP_VERSION}\n"
runcmd:
  - usermod -aG docker {SERVER_USER}
  - systemctl enable --now docker unattended-upgrades
  - systemctl restart ssh || systemctl restart sshd
"#,
        host_private = indent(setup.host_private_key.expose()),
        host_public = setup.host_public_key,
        authorized = setup.authorized_key,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smallest_current_type_is_picked() {
        let types = json!({"server_types": [
            {"name": "cx53", "architecture": "x86", "memory": 32.0, "cores": 16, "deprecation": null, "prices": [{"location": "fsn1"}]},
            {"name": "cx11", "architecture": "x86", "memory": 2.0, "cores": 1, "deprecated": true, "deprecation": {"announced": "2024"}, "prices": [{"location": "fsn1"}]},
            {"name": "cax11", "architecture": "arm", "memory": 4.0, "cores": 2, "deprecation": null, "prices": [{"location": "fsn1"}]},
            {"name": "cx23", "architecture": "x86", "memory": 4.0, "cores": 2, "deprecation": null, "prices": [{"location": "fsn1"}, {"location": "nbg1"}]},
            {"name": "cpx11", "architecture": "x86", "memory": 2.0, "cores": 2, "deprecation": null, "prices": [{"location": "ash"}]},
        ]});
        assert_eq!(pick_smallest(&types, Some("fsn1")).as_deref(), Some("cx23"));
        assert_eq!(pick_smallest(&types, None).as_deref(), Some("cpx11"));
        assert_eq!(pick_smallest(&types, Some("sin")), None);
    }

    #[test]
    fn first_boot_locks_the_server_down() {
        let setup = ServerSetup::generate("web-1").unwrap();
        let config = cloud_init("web-1", &setup);
        assert!(config.starts_with("#cloud-config\n"));
        assert!(config.contains("PermitRootLogin no"));
        assert!(config.contains("PasswordAuthentication no"));
        assert!(config.contains(&format!("      - {}", setup.authorized_key)));
        assert!(config.contains(&format!("  ed25519_public: {}", setup.host_public_key)));
        // The host key sits indented under its block, every line of it.
        for line in setup.host_private_key.expose().lines() {
            assert!(config.contains(&format!("\n    {line}\n")), "{line}");
        }
    }

    #[test]
    fn retries_label_alike_and_names_are_checked() {
        let setup = ServerSetup::generate("web-1").unwrap();
        let what = ProviderNew::Server {
            name: "web-1",
            server_type: None,
            location: Some("fsn1"),
            volume_gb: None,
            backups: false,
            setup: Some(&setup),
        };
        let one = NewServer::from(&what, "shop").unwrap().unwrap();
        let two = NewServer::from(&what, "shop").unwrap().unwrap();
        assert_eq!(one.mark(), two.mark());
        assert_eq!(one.server_labels()[PROJECT_LABEL], "shop");
        // A project name Hetzner won't take as a label is left off.
        let odd = NewServer::from(&what, "My shop!").unwrap().unwrap();
        assert!(odd.server_labels().get(PROJECT_LABEL).is_none());
        let bad = ProviderNew::Server {
            name: "web-1",
            server_type: Some("cx23; rm -rf /"),
            location: None,
            volume_gb: None,
            backups: false,
            setup: Some(&setup),
        };
        assert!(NewServer::from(&bad, "shop").is_err());
        let keyless = ProviderNew::Server {
            name: "web-1",
            server_type: None,
            location: None,
            volume_gb: None,
            backups: false,
            setup: None,
        };
        assert!(NewServer::from(&keyless, "shop").is_err());
    }
}
