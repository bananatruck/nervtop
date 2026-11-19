use super::{RateBaseline, Source, entries, number};
use crate::model::{NetInterface, Reading};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Instant,
};

pub struct Network {
    root: PathBuf,
    rates: BTreeMap<(u32, String), (RateBaseline, RateBaseline)>,
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

impl Source for Network {
    type Output = Vec<NetInterface>;

    fn sample(&mut self) -> Reading<Self::Output> {
        let now = Instant::now();
        let mut seen = BTreeSet::new();
        let mut interfaces = Vec::new();
        for dir in entries(self.root.join("sys/class/net")) {
            let Some(name) = dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            let Some(ifindex) = number::<u32>(dir.join("ifindex")) else {
                continue;
            };
            if name == "lo" {
                continue;
            }
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
                name,
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
    use std::fs;

    fn write_interface(root: &std::path::Path, name: &str, ifindex: u32, rx: u64, tx: u64) {
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

        write_interface(root.path(), "eth0", 8, 5, 9);
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
}
