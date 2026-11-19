use super::{Rates, Source, entries, number};
use crate::model::{NetInterface, Reading};
use std::{collections::BTreeMap, path::PathBuf};

pub struct Network {
    root: PathBuf,
    rates: Rates<(u32, String)>,
}

impl Network {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            rates: Rates::default(),
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
        let mut counters = BTreeMap::new();
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
            let Some(rx) = number::<u64>(dir.join("statistics/rx_bytes")) else {
                continue;
            };
            let Some(tx) = number::<u64>(dir.join("statistics/tx_bytes")) else {
                continue;
            };
            if name != "lo" && (rx > 0 || tx > 0) {
                counters.insert((ifindex, name), (rx, tx));
            }
        }
        let rates = self.rates.sample(counters);
        let mut interfaces: Vec<_> = rates
            .into_iter()
            .map(|((ifindex, name), (rx_bps, tx_bps))| {
                let (rx_total_bytes, tx_total_bytes) =
                    self.rates.previous[&(ifindex, name.clone())];
                NetInterface {
                    name,
                    rx_bps,
                    tx_bps,
                    rx_total_bytes,
                    tx_total_bytes,
                }
            })
            .collect();
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
}
