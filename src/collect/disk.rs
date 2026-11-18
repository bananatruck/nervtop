use super::{Rates, Source, read};
use crate::model::{DiskStats, Reading};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use sysinfo::DiskRefreshKind;
pub struct Disks {
    disks: Option<sysinfo::Disks>,
    root: PathBuf,
    rates: Rates,
}
impl Disks {
    pub fn new(root: PathBuf) -> Self {
        Self {
            disks: sysinfo::Disks::new().ok(),
            root,
            rates: Rates::default(),
        }
    }
}
// Linux iostats: sectors read/written are stats 3/7 after the name,
// hence columns 6/10 in /proc/diskstats including major, minor and name.
// https://docs.kernel.org/admin-guide/iostats.html
fn parse_diskstats(text: &str) -> BTreeMap<String, (u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            let name = *f.get(2)?;
            let read = f.get(5)?.parse::<u64>().ok()?.checked_mul(512)?;
            let write = f.get(9)?.parse::<u64>().ok()?.checked_mul(512)?;
            Some((name.to_owned(), (read, write)))
        })
        .collect()
}
fn device_name(root: &Path, device: &Path) -> Option<String> {
    let path = root.join(device.strip_prefix("/").ok()?);
    let resolved = std::fs::canonicalize(&path).unwrap_or(path);
    resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}
impl Source for Disks {
    type Output = Vec<DiskStats>;
    fn sample(&mut self) -> Reading<Self::Output> {
        let Some(disks) = &mut self.disks else {
            return Reading::unavailable("disk initialization failed");
        };
        disks.refresh_specifics(true, DiskRefreshKind::nothing().with_storage());
        let counters = read(self.root.join("proc/diskstats"));
        let rates = self
            .rates
            .sample(counters.as_deref().map(parse_diskstats).unwrap_or_default());
        let mut out: BTreeMap<String, DiskStats> = BTreeMap::new();
        for disk in disks.iter() {
            let path = Path::new(disk.name());
            if !path.starts_with("/dev") {
                continue;
            }
            let Some(device) = device_name(&self.root, path) else {
                continue;
            };
            let mount_point = disk.mount_point().to_string_lossy().into_owned();
            if out
                .get(&device)
                .is_some_and(|d| d.mount_point.len() <= mount_point.len())
            {
                continue;
            }
            let missing = if counters.is_none() {
                "diskstats not readable"
            } else {
                "device counters not reported"
            };
            let (read_bps, write_bps) = rates
                .get(&device)
                .cloned()
                .unwrap_or_else(|| (Reading::unavailable(missing), Reading::unavailable(missing)));
            out.insert(
                device.clone(),
                DiskStats {
                    mount_point,
                    device,
                    file_system: disk.file_system().to_string_lossy().into_owned(),
                    total_bytes: disk.total_space(),
                    available_bytes: disk.available_space(),
                    read_bps,
                    write_bps,
                },
            );
        }
        let mut out: Vec<_> = out.into_values().collect();
        out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
        Reading::Available(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_diskstats_columns() {
        let stats = parse_diskstats(
            "259 0 nvme0n1 255999 814 12369153 47919 996852 81 36123024 425995 0 301795 580470 0 0 0 0 60602 106555\n253 0 dm-0 1031 0 8248 250 406 0 3248 35 0 200 285\nbad line\n",
        );
        assert_eq!(stats["nvme0n1"], (12369153 * 512, 36123024 * 512));
        assert_eq!(stats["dm-0"], (8248 * 512, 3248 * 512));
        assert_eq!(stats.len(), 2);
    }
    #[test]
    fn mapper_symlink_resolves_to_kernel_name() {
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree.path().join("dev/mapper")).unwrap();
        std::fs::write(tree.path().join("dev/dm-0"), "").unwrap();
        std::os::unix::fs::symlink("../dm-0", tree.path().join("dev/mapper/root")).unwrap();
        assert_eq!(
            device_name(tree.path(), Path::new("/dev/mapper/root")),
            Some("dm-0".into())
        );
    }
}
