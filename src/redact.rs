//! Masks identifying names in snapshots for shareable output (`--redact`).
//!
//! Every number stays real. Only names change: the hostname, user names
//! other than system accounts, process names outside a short list of generic
//! system and developer programs, network interface names other than
//! standard ones, and personal paths in mount points. Labels are stable for
//! the life of a `Redactor`, so the same process keeps the same label across
//! samples.

use crate::model::{Reading, Snapshot};
use std::collections::HashMap;

/// The hostname shown in redacted output.
pub const HOSTNAME: &str = "nervtop-demo";

/// Process names shown unchanged: generic system and developer programs.
const GENERIC_PROCESSES: &[&str] = &[
    "systemd",
    "systemd-journal",
    "systemd-logind",
    "systemd-udevd",
    "systemd-userdbd",
    "systemd-timesyn",
    "systemd-resolve",
    "systemd-oomd",
    "dbus-broker",
    "dbus-broker-lau",
    "dbus-daemon",
    "NetworkManager",
    "gnome-shell",
    "Xwayland",
    "gdm",
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "polkitd",
    "bluetoothd",
    "sshd",
    "bash",
    "zsh",
    "sh",
    "tmux",
    "cargo",
    "rustc",
    "nervtop",
    "chrome",
    "firefox",
    "node",
    "python3",
    "accounts-daemon",
    "boltd",
    "upowerd",
    "kthreadd",
];

/// System accounts shown unchanged; every other user becomes "user".
const SYSTEM_USERS: &[&str] = &[
    "root", "dbus", "polkitd", "gdm", "nobody", "avahi", "rtkit", "colord", "geoclue",
];

#[derive(Debug, Default)]
pub struct Redactor {
    processes: HashMap<String, String>,
    interfaces: HashMap<String, String>,
}

impl Redactor {
    /// Rewrites the names in `s` in place; numbers are untouched.
    pub fn apply(&mut self, s: &mut Snapshot) {
        if s.host.hostname.is_available() {
            s.host.hostname = Reading::Available(HOSTNAME.to_owned());
        }
        if let Reading::Available(procs) = &mut s.processes {
            for p in procs.iter_mut() {
                p.name = self.process_name(&p.name, p.kernel_thread);
                if !is_system_user(&p.user) {
                    p.user = "user".to_owned();
                }
            }
        }
        if let Reading::Available(ifaces) = &mut s.network {
            for n in ifaces.iter_mut() {
                n.name = self.interface_name(&n.name);
            }
        }
        if let Reading::Available(disks) = &mut s.disks {
            for d in disks.iter_mut() {
                d.mount_point = mask_home(&d.mount_point);
            }
        }
    }

    fn process_name(&mut self, name: &str, kernel_thread: bool) -> String {
        if kernel_thread || GENERIC_PROCESSES.contains(&name) {
            return name.to_owned();
        }
        let next = self.processes.len() + 1;
        self.processes
            .entry(name.to_owned())
            .or_insert_with(|| format!("app-{next:02}"))
            .clone()
    }

    fn interface_name(&mut self, name: &str) -> String {
        if is_standard_interface(name) {
            return name.to_owned();
        }
        let next = self.interfaces.len() + 1;
        self.interfaces
            .entry(name.to_owned())
            .or_insert_with(|| format!("net-{next}"))
            .clone()
    }
}

fn is_system_user(user: &str) -> bool {
    SYSTEM_USERS.contains(&user) || user.starts_with("systemd-")
}

/// `lo`, `ethN`, `wlanN` and systemd predictable names (`enp3s0`, `wlp2s0`).
fn is_standard_interface(name: &str) -> bool {
    let digits_after = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    };
    name == "lo"
        || digits_after("eth")
        || digits_after("wlan")
        || ((name.starts_with("en") || name.starts_with("wl"))
            && name.len() <= 15
            && name.chars().all(|c| c.is_ascii_alphanumeric())
            && name.chars().any(|c| c.is_ascii_digit()))
}

/// `/home/<name>/...` becomes `/home/user/...`.
fn mask_home(mount: &str) -> String {
    let mut parts = mount.splitn(4, '/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(""), Some("home"), Some(_), rest) => match rest {
            Some(rest) => format!("/home/user/{rest}"),
            None => "/home/user".to_owned(),
        },
        _ => mount.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::fake;
    use crate::model::*;

    #[test]
    fn names_are_masked_and_numbers_kept() {
        let mut s = fake::sample_snapshot();
        let template = s.processes.value().unwrap()[0].clone();
        s.processes = Reading::Available(vec![
            ProcessInfo {
                pid: 10,
                name: "secret-tool".into(),
                user: "kesh".into(),
                cpu_percent: 42.0,
                ..template.clone()
            },
            ProcessInfo {
                pid: 11,
                name: "systemd".into(),
                user: "root".into(),
                ..template.clone()
            },
            ProcessInfo {
                pid: 12,
                name: "secret-tool".into(),
                user: "kesh".into(),
                ..template.clone()
            },
        ]);
        let iface = s.network.value().unwrap()[0].clone();
        s.network = Reading::Available(vec![
            NetInterface {
                name: "tailscale0".into(),
                ..iface.clone()
            },
            NetInterface {
                name: "wlp2s0".into(),
                ..iface.clone()
            },
        ]);
        let disk = s.disks.value().unwrap()[0].clone();
        s.disks = Reading::Available(vec![DiskStats {
            mount_point: "/home/kesh/data".into(),
            ..disk
        }]);
        let mut r = Redactor::default();
        r.apply(&mut s);
        let p = s.processes.value().unwrap();
        assert_eq!(p[0].name, "app-01");
        assert_eq!(p[0].user, "user");
        assert_eq!(p[0].cpu_percent, 42.0);
        assert_eq!(p[1].name, "systemd");
        assert_eq!(p[1].user, "root");
        assert_eq!(p[2].name, "app-01", "same name, same label");
        let n = s.network.value().unwrap();
        assert_eq!(n[0].name, "net-1");
        assert_eq!(n[1].name, "wlp2s0");
        assert_eq!(s.disks.value().unwrap()[0].mount_point, "/home/user/data");
        assert_eq!(s.host.hostname, Reading::Available(HOSTNAME.into()));
        let json = serde_json::to_string(&s).unwrap();
        for leak in ["kesh", "secret-tool", "tailscale"] {
            assert!(!json.contains(leak), "{leak} leaked");
        }
    }

    #[test]
    fn standard_interfaces_and_mounts() {
        for n in ["lo", "eth0", "wlan0", "enp3s0", "wlp2s0"] {
            assert!(is_standard_interface(n), "{n}");
        }
        for n in [
            "tailscale0",
            "veth5d9f83b",
            "br-7237d48b080b",
            "docker0",
            "eth",
        ] {
            assert!(!is_standard_interface(n), "{n}");
        }
        assert_eq!(mask_home("/"), "/");
        assert_eq!(mask_home("/boot"), "/boot");
        assert_eq!(mask_home("/home/kesh"), "/home/user");
    }
}
