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
    "root",
    "dbus",
    "polkitd",
    "gdm",
    "nobody",
    "avahi",
    "rtkit",
    "colord",
    "geoclue",
    "systemd-network",
    "systemd-resolve",
    "systemd-timesync",
    "systemd-journal",
    "systemd-journal-remote",
    "systemd-oom",
    "systemd-coredump",
];

/// Mount points shown unchanged; any other becomes `/mnt/disk-N`, because
/// paths such as `/home/<name>` or `/run/media/<name>/<label>` identify
/// people and projects.
const STANDARD_MOUNTS: &[&str] = &[
    "/",
    "/boot",
    "/boot/efi",
    "/efi",
    "/home",
    "/var",
    "/tmp",
    "/usr",
    "/opt",
    "/srv",
    "/nix",
];

#[derive(Debug, Default)]
pub struct Redactor {
    processes: HashMap<String, String>,
    interfaces: HashMap<String, String>,
    mounts: HashMap<String, String>,
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
                d.mount_point = self.mount_point(&d.mount_point);
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

impl Redactor {
    /// Standard mount points unchanged, every other one a distinct stable alias.
    pub fn mount_point(&mut self, mount: &str) -> String {
        if STANDARD_MOUNTS.contains(&mount) {
            return mount.to_owned();
        }
        let next = self.mounts.len() + 1;
        self.mounts
            .entry(mount.to_owned())
            .or_insert_with(|| format!("/mnt/disk-{next}"))
            .clone()
    }
}

fn is_system_user(user: &str) -> bool {
    SYSTEM_USERS.contains(&user)
}

/// `lo`, `ethN`, `wlanN`, and systemd predictable names built only from
/// bus positions (`eno1`, `ens33`, `enp0s31f6`, `wlp2s0`, `enp5s0u1`).
/// After the `en`, `wl` or `ww` prefix, such a name is a sequence of marker
/// letters (systemd.net-naming-scheme(7): `o`, `s`, `p`, `P`, `v`, `b` first,
/// then `f`, `n`, `d`, `u`, `c`, `i`), each followed by its number, so
/// letters from that alphabet cannot spell a word (`ens1finn`). MAC-based
/// names (`enx001122aabbcc`), port names after `n` that are not numbers, and
/// anything else are masked.
fn is_standard_interface(name: &str) -> bool {
    let digits_after = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    };
    if name == "lo" || digits_after("eth") || digits_after("wlan") {
        return true;
    }
    let Some(rest) = ["en", "wl", "ww"].iter().find_map(|p| name.strip_prefix(p)) else {
        return false;
    };
    if !rest.starts_with(['o', 's', 'p', 'P', 'v', 'b']) {
        return false;
    }
    let mut chars = rest.chars().peekable();
    while let Some(marker) = chars.next() {
        if !"ospPvbfndcui".contains(marker) {
            return false;
        }
        let mut digits = 0;
        while chars.next_if(char::is_ascii_digit).is_some() {
            digits += 1;
        }
        if digits == 0 {
            return false;
        }
    }
    true
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
                cpu_percent: Reading::Available(42.0),
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
        assert_eq!(p[0].cpu_percent, Reading::Available(42.0));
        assert_eq!(p[1].name, "systemd");
        assert_eq!(p[1].user, "root");
        assert_eq!(p[2].name, "app-01", "same name, same label");
        let n = s.network.value().unwrap();
        assert_eq!(n[0].name, "net-1");
        assert_eq!(n[1].name, "wlp2s0");
        assert_eq!(s.disks.value().unwrap()[0].mount_point, "/mnt/disk-1");
        assert_eq!(s.host.hostname, Reading::Available(HOSTNAME.into()));
        let json = serde_json::to_string(&s).unwrap();
        for leak in ["kesh", "secret-tool", "tailscale"] {
            assert!(!json.contains(leak), "{leak} leaked");
        }
    }

    #[test]
    fn identifiers_hidden_in_standard_looking_names_are_masked() {
        // Every marker letter must carry a number, so letters chosen from
        // the marker alphabet cannot spell a name ("ens1finn").
        for n in [
            "enx001122aabbcc",
            "enalice1",
            "wlbob2",
            "ens1finn",
            "enpips1",
            "eno1nic",
            "ens1f",
            "enp0s",
            "ens01d",
        ] {
            assert!(!is_standard_interface(n), "{n}");
        }
        for n in [
            "enp0s31f6",
            "wlp0s20f3",
            "eno1",
            "ens33",
            "enp5s0u1",
            "enP1p0s0",
            "enp0s20u1c2i3",
            "wwp0s20f0u2i12",
            "eno1d1",
            "env1",
            "enb3",
        ] {
            assert!(is_standard_interface(n), "{n}");
        }
        assert!(!is_system_user("systemd-alice"));
        assert!(is_system_user("systemd-timesync"));
    }

    #[test]
    fn distinct_private_mounts_get_distinct_aliases() {
        let mut r = Redactor::default();
        assert_eq!(r.mount_point("/home/alice"), "/mnt/disk-1");
        assert_eq!(r.mount_point("/run/media/alice/ClientAcme"), "/mnt/disk-2");
        assert_eq!(r.mount_point("/home/alice"), "/mnt/disk-1", "stable");
        assert_eq!(r.mount_point("/boot/efi"), "/boot/efi");
        assert_eq!(r.mount_point("/"), "/");
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
    }
}
