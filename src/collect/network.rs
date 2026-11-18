use super::{Rates, Source};
use crate::model::{NetInterface, Reading};
pub struct Network {
    networks: Option<sysinfo::Networks>,
    rates: Rates,
}
impl Default for Network {
    fn default() -> Self {
        Self {
            networks: sysinfo::Networks::new().ok(),
            rates: Rates::default(),
        }
    }
}
impl Source for Network {
    type Output = Vec<NetInterface>;
    fn sample(&mut self) -> Reading<Self::Output> {
        let Some(networks) = &mut self.networks else {
            return Reading::unavailable("network initialization failed");
        };
        networks.refresh(true);
        let counters = networks
            .iter()
            .filter(|(name, n)| {
                name.as_os_str() != "lo" && (n.total_received() > 0 || n.total_transmitted() > 0)
            })
            .map(|(name, n)| {
                (
                    name.to_string_lossy().into_owned(),
                    (n.total_received(), n.total_transmitted()),
                )
            })
            .collect();
        let rates = self.rates.sample(counters);
        Reading::Available(
            rates
                .into_iter()
                .map(|(name, (rx_bps, tx_bps))| {
                    let (rx_total_bytes, tx_total_bytes) = self.rates.previous[&name];
                    NetInterface {
                        name,
                        rx_bps,
                        tx_bps,
                        rx_total_bytes,
                        tx_total_bytes,
                    }
                })
                .collect(),
        )
    }
}
