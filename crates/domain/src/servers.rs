//! Rules for what agents do on a server through Hoista: the commands and
//! files that always wait for the person, and the files nobody reads.

use regex::Regex;
use std::sync::OnceLock;

/// Commands that can't be undone or that can lock people out: deleting
/// system folders, formatting disks, firewall, SSH, users and keys, reboots.
const ALWAYS_ASK: &[&str] = &[
    // rm -r or -f on /, anything in a system folder, or a home or app
    // folder itself (what's inside an app's folder is fine).
    r"\brm\s+(?:-\S*\s+)*-\S*[rRf]\S*\s+(?:\S+\s+)*/(?:\s|$|\*|(?:bin|boot|dev|etc|lib|lib32|lib64|proc|root|run|sbin|sys|usr|var)(?:/|\s|$|\*)|(?:home|opt|srv)(?:/[^/\s]+)?/?(?:\s|$|\*))",
    r"\b(?:mkfs(?:\.\w+)?|fdisk|sfdisk|gdisk|parted|wipefs|shred|mkswap)\b",
    r"\bdd\b[^\n]*\bof=/dev/",
    r">\s*/dev/(?:sd|vd|nvme|xvd)",
    r"\b(?:ufw|iptables|ip6tables|nft|firewall-cmd)\b",
    r"\bsshd\b|/etc/ssh\b|\bauthorized_keys\b|\bssh-keygen\b",
    r"\b(?:useradd|userdel|usermod|adduser|deluser|addgroup|delgroup|groupadd|groupdel|gpasswd|passwd|chpasswd|chage|visudo)\b",
    r"/etc/(?:sudoers|passwd|shadow|group|gshadow)\b",
    r"\b(?:reboot|shutdown|poweroff|halt)\b",
    r"\bsystemctl\s+(?:\S+\s+)*(?:reboot|poweroff|halt|kexec|emergency|rescue)\b",
    r"\binit\s+[06]\b",
];

/// Files whose contents stay on the server: password hashes, private keys
/// and other processes' environments.
const NEVER_READ: &[&str] = &[
    r"^/etc/(?:shadow|gshadow)(?:-|\.|$)",
    r"^/etc/sudoers(?:\.d)?(?:/|$)",
    r"^/etc/ssh/ssh_host_[^/]*_key$",
    r"/\.ssh/id_[^/]*$",
    r"/\.ssh/[^/]*_ed25519$|/\.ssh/[^/]*_rsa$|/\.ssh/[^/]*_ecdsa$",
    r"\.(?:key|pem|p12|pfx)$",
    r"^/proc/[^/]+/environ$",
    r"^/root/",
];

/// Files that change who can sign in or what reaches the server; writing one
/// always waits for the person.
const ALWAYS_ASK_WRITE: &[&str] = &[
    r"^/etc/ssh/",
    r"/\.ssh/",
    r"^/etc/(?:sudoers|passwd|shadow|group|gshadow)(?:\.d)?(?:/|$|-)",
    r"^/etc/(?:ufw|nftables|iptables)",
    r"^/etc/pam\.d/",
    r"^/boot/",
];

fn compiled(cell: &'static OnceLock<Vec<Regex>>, patterns: &[&str]) -> &'static [Regex] {
    cell.get_or_init(|| {
        patterns
            .iter()
            .filter_map(|pattern| Regex::new(pattern).ok())
            .collect()
    })
}

/// Whether `command` is on the always-ask list.
#[must_use]
pub fn command_always_asks(command: &str) -> bool {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    compiled(&CELL, ALWAYS_ASK)
        .iter()
        .any(|pattern| pattern.is_match(command))
}

/// Whether the file at `path` (absolute, with links followed) is one Hoista
/// never reads for an agent.
#[must_use]
pub fn path_never_read(path: &str) -> bool {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    compiled(&CELL, NEVER_READ)
        .iter()
        .any(|pattern| pattern.is_match(path))
}

/// Whether writing the file at `path` always waits for the person.
#[must_use]
pub fn write_always_asks(path: &str) -> bool {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    compiled(&CELL, ALWAYS_ASK_WRITE)
        .iter()
        .any(|pattern| pattern.is_match(path))
}

/// Whether `path` is a plain absolute path: no `..`, no control characters,
/// nothing the shell reads specially once quoted.
#[must_use]
pub fn plain_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 1024
        && !path.split('/').any(|part| part == "..")
        && !path.chars().any(char::is_control)
}

/// `text` quoted for a POSIX shell: inside single quotes, with each single
/// quote closed, escaped and reopened.
#[must_use]
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risky_commands_always_ask() {
        for command in [
            "rm -rf /",
            "sudo rm -rf /etc/nginx",
            "rm -r -f /var/lib/docker",
            "mkfs.ext4 /dev/sdb",
            "dd if=/dev/zero of=/dev/sda bs=1M",
            "sudo ufw allow 8080",
            "iptables -F",
            "sudo systemctl restart sshd",
            "echo key >> ~/.ssh/authorized_keys",
            "sudo useradd deploy",
            "sudo passwd hoista",
            "sudo reboot",
            "sudo systemctl reboot",
            "shutdown -h now",
            "sudo vi /etc/sudoers",
        ] {
            assert!(command_always_asks(command), "{command}");
        }
    }

    #[test]
    fn everyday_commands_dont() {
        for command in [
            "docker compose up -d",
            "rm -rf ./build",
            "rm -rf /home/hoista/app/node_modules",
            "sudo systemctl restart caddy",
            "git pull && docker compose build",
            "cat /etc/os-release",
            "apt list --upgradable",
            "ls -la /var/log",
        ] {
            assert!(!command_always_asks(command), "{command}");
        }
    }

    #[test]
    fn secrets_are_never_read() {
        for path in [
            "/etc/shadow",
            "/etc/sudoers.d/90-cloud-init-users",
            "/etc/ssh/ssh_host_ed25519_key",
            "/home/hoista/.ssh/id_ed25519",
            "/srv/app/tls.key",
            "/proc/1/environ",
            "/root/.bash_history",
        ] {
            assert!(path_never_read(path), "{path}");
        }
        for path in [
            "/etc/ssh/ssh_host_ed25519_key.pub",
            "/etc/os-release",
            "/srv/app/compose.yaml",
            "/home/hoista/.ssh/authorized_keys",
        ] {
            assert!(!path_never_read(path), "{path}");
        }
    }

    #[test]
    fn writes_that_change_access_ask() {
        assert!(write_always_asks("/etc/ssh/sshd_config"));
        assert!(write_always_asks("/home/hoista/.ssh/authorized_keys"));
        assert!(write_always_asks("/etc/sudoers.d/deploy"));
        assert!(!write_always_asks("/srv/app/compose.yaml"));
        assert!(!write_always_asks("/etc/caddy/Caddyfile"));
    }

    #[test]
    fn paths_and_quoting() {
        assert!(plain_path("/srv/app/compose.yaml"));
        assert!(!plain_path("srv/app"));
        assert!(!plain_path("/srv/../etc/shadow"));
        assert!(!plain_path("/srv/a\nb"));
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
