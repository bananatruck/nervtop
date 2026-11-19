use super::{RateBaseline, Source, number};
use crate::model::{NetInterface, Reading};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    time::Instant,
};

pub struct Network {
    root: PathBuf,
    rates: BTreeMap<(u32, OsString), (RateBaseline, RateBaseline)>,
}

impl Network {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            rates: BTreeMap::new(),
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Self::new("/".into())
    }
}

fn display_name(name: &OsStr) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut display = String::new();
    for byte in name.as_bytes() {
        match byte {
            b'\\' => display.push_str("\\\\"),
            b' '..=b'~' => display.push(char::from(*byte)),
            _ => {
                display.push_str("\\x");
                display.push(char::from(HEX[(byte >> 4) as usize]));
                display.push(char::from(HEX[(byte & 0xf) as usize]));
            }
        }
    }
    display
}

impl Source for Network {
    type Output = Vec<NetInterface>;

    fn sample(&mut self) -> Reading<Self::Output> {
        let now = Instant::now();
        let mut seen = BTreeSet::new();
        let mut interfaces = Vec::new();
        let Ok(dirs) = std::fs::read_dir(self.root.join("sys/class/net")) else {
            return Reading::unavailable("network inventory not readable");
        };
        for dir in dirs {
            let Ok(dir) = dir else {
                return Reading::unavailable("network inventory not readable");
            };
            let name = dir.file_name();
            if name == OsStr::new("lo") {
                continue;
            }
            let dir = dir.path();
            let Some(ifindex) = number::<u32>(dir.join("ifindex")) else {
                return Reading::unavailable("interface identity not readable");
            };
            let rx = number::<u64>(dir.join("statistics/rx_bytes"));
            let tx = number::<u64>(dir.join("statistics/tx_bytes"));
            if rx == Some(0) && tx == Some(0) {
                continue;
            }
            let key = (ifindex, name.clone());
            seen.insert(key.clone());
            let (rx_baseline, tx_baseline) = self.rates.entry(key).or_default();
            let rx_bps = rx_baseline.sample(rx, now);
            let tx_bps = tx_baseline.sample(tx, now);
            interfaces.push(NetInterface {
                name: display_name(&name),
                rx_bps,
                tx_bps,
                rx_total_bytes: rx_baseline.latest().unwrap_or(0),
                tx_total_bytes: tx_baseline.latest().unwrap_or(0),
            });
        }
        self.rates.retain(|key, _| seen.contains(key));
        interfaces.sort_by(|a, b| a.name.cmp(&b.name));
        Reading::Available(interfaces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsStr, fs, os::unix::ffi::OsStringExt};

    fn write_interface(root: &std::path::Path, name: &str, ifindex: u32, rx: u64, tx: u64) {
        write_interface_name(root, OsStr::new(name), ifindex, rx, tx);
    }

    fn write_interface_name(root: &std::path::Path, name: &OsStr, ifindex: u32, rx: u64, tx: u64) {
        let dir = root.join("sys/class/net").join(name);
        fs::create_dir_all(dir.join("statistics")).unwrap();
        fs::write(dir.join("ifindex"), ifindex.to_string()).unwrap();
        fs::write(dir.join("statistics/rx_bytes"), rx.to_string()).unwrap();
        fs::write(dir.join("statistics/tx_bytes"), tx.to_string()).unwrap();
    }

    #[test]
    fn replacement_interface_warms_up_again() {
        let root = tempfile::tempdir().unwrap();
        write_interface(root.path(), "eth0", 7, 100, 200);
        let mut network = Network::new(root.path().into());

        let first_sample = network.sample();
        let first = first_sample.value().unwrap();
        assert_eq!(first[0].rx_bps.reason(), Some("warming up"));

        write_interface(root.path(), "eth0", 7, 110, 220);
        let second_sample = network.sample();
        let second = second_sample.value().unwrap();
        assert!(second[0].rx_bps.is_available());
        assert!(second[0].tx_bps.is_available());

        write_interface(root.path(), "eth0", 8, 1100, 2200);
        let replacement_sample = network.sample();
        let replacement = replacement_sample.value().unwrap();
        assert_eq!(replacement[0].rx_bps.reason(), Some("warming up"));
        assert_eq!(replacement[0].tx_bps.reason(), Some("warming up"));
    }

    #[test]
    fn unreadable_direction_never_becomes_a_baseline() {
        let root = tempfile::tempdir().unwrap();
        write_interface(root.path(), "eth0", 7, 1, 100);
        let rx_path = root.path().join("sys/class/net/eth0/statistics/rx_bytes");
        fs::write(&rx_path, "not-a-counter").unwrap();
        let mut network = Network::new(root.path().into());

        let unreadable_sample = network.sample();
        let unreadable = &unreadable_sample.value().unwrap()[0];
        assert_eq!(unreadable.rx_bps.reason(), Some("counter not readable"));
        assert_eq!(unreadable.tx_bps.reason(), Some("warming up"));

        write_interface(root.path(), "eth0", 7, 10, 110);
        let first_valid_sample = network.sample();
        let first_valid = &first_valid_sample.value().unwrap()[0];
        assert_eq!(first_valid.rx_bps.reason(), Some("warming up"));
        assert!(first_valid.tx_bps.is_available());

        write_interface(root.path(), "eth0", 7, 20, 120);
        let established_sample = network.sample();
        assert!(established_sample.value().unwrap()[0].rx_bps.is_available());

        fs::write(&rx_path, "still-not-a-counter").unwrap();
        fs::write(
            root.path().join("sys/class/net/eth0/statistics/tx_bytes"),
            "130",
        )
        .unwrap();
        let failed_sample = network.sample();
        let failed = &failed_sample.value().unwrap()[0];
        assert_eq!(failed.rx_bps.reason(), Some("counter not readable"));
        assert!(failed.tx_bps.is_available());

        write_interface(root.path(), "eth0", 7, 40, 140);
        let recovered_sample = network.sample();
        assert!(recovered_sample.value().unwrap()[0].rx_bps.is_available());
    }

    #[test]
    fn non_utf8_interface_names_stay_distinct_and_escaped() {
        let root = tempfile::tempdir().unwrap();
        let first = std::ffi::OsString::from_vec(b"eth\xfe".to_vec());
        let second = std::ffi::OsString::from_vec(b"eth\xff".to_vec());
        write_interface_name(root.path(), &first, 7, 10, 20);
        write_interface_name(root.path(), &second, 8, 30, 40);
        let mut network = Network::new(root.path().into());

        let sample = network.sample();
        let names: Vec<_> = sample
            .value()
            .unwrap()
            .iter()
            .map(|interface| interface.name.as_str())
            .collect();

        assert_eq!(names, ["eth\\xfe", "eth\\xff"]);
    }
    #[test]
    fn unreadable_interface_inventory_is_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let mut network = Network::new(root.path().into());
        assert_eq!(
            network.sample().reason(),
            Some("network inventory not readable")
        );
        write_interface(root.path(), "eth0", 7, 10, 20);
        assert!(network.sample().is_available());
        fs::remove_file(root.path().join("sys/class/net/eth0/ifindex")).unwrap();
        assert_eq!(
            network.sample().reason(),
            Some("interface identity not readable")
        );
    }
}
