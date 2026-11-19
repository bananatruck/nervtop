use super::{Rates, Source, read};
use crate::model::{DiskStats, Reading};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Total and available bytes of the filesystem mounted at a path.
type SpaceFn = dyn FnMut(&Path) -> Option<(u64, u64)> + Send;

pub struct Disks {
    root: PathBuf,
    rates: Rates,
    space: Box<SpaceFn>,
}
impl Disks {
    pub fn new(root: PathBuf) -> Self {
        Self::with_space(root, statvfs_space)
    }
    /// Uses `space` instead of statvfs(3), so tests can see which mounts are queried.
    pub fn with_space(
        root: PathBuf,
        space: impl FnMut(&Path) -> Option<(u64, u64)> + Send + 'static,
    ) -> Self {
        Self {
            root,
            rates: Rates::default(),
            space: Box::new(space),
        }
    }
}

/// statvfs(3) on `path`: total and available-to-users bytes.
fn statvfs_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated path and `st` is plain data that
    // statvfs fills in completely when it returns 0.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let unit = st.f_frsize as u64;
    Some((
        (st.f_blocks as u64).checked_mul(unit)?,
        (st.f_bavail as u64).checked_mul(unit)?,
    ))
}

/// Decodes the octal escapes `/proc/self/mounts` uses for spaces, tabs,
/// newlines and backslashes in paths.
fn unescape_mount(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let v = (bytes[i + 1] - b'0') as u32 * 64
                + (bytes[i + 2] - b'0') as u32 * 8
                + (bytes[i + 3] - b'0') as u32;
            if let Ok(b) = u8::try_from(v) {
                out.push(b);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `(device, mount point, filesystem type)` for each visible mount in a
/// mounts table. Fields are separated by ASCII spaces and tabs only, so a
/// no-break space inside a path survives; a record that is not valid UTF-8 is
/// skipped on its own; and a mount covered by a later mount at the same path
/// is dropped, because querying that path would reach the covering one.
fn parse_mounts(table: &[u8]) -> Vec<(String, String, String)> {
    let records: Vec<(String, String, String)> = table
        .split(|b| *b == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| {
            let mut f = line.split([' ', '\t']).filter(|x| !x.is_empty());
            let device = unescape_mount(f.next()?);
            let mount = unescape_mount(f.next()?);
            let fs = f.next()?.to_owned();
            Some((device, mount, fs))
        })
        .collect();
    records
        .iter()
        .enumerate()
        .filter(|(i, (_, mount, _))| !records[i + 1..].iter().any(|(_, m, _)| m == mount))
        .map(|(_, r)| r.clone())
        .collect()
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
        let Ok(mounts) = std::fs::read(self.root.join("proc/self/mounts")) else {
            return Reading::unavailable("mount table not readable");
        };
        let counters = read(self.root.join("proc/diskstats"));
        let rates = self
            .rates
            .sample(counters.as_deref().map(parse_diskstats).unwrap_or_default());
        // Filter and deduplicate before asking for space: statvfs on a FUSE or
        // network mount whose server has gone away can block indefinitely, so
        // only block-device mounts are ever queried.
        let mut chosen: BTreeMap<String, (String, String)> = BTreeMap::new();
        for (dev, mount_point, fs) in parse_mounts(&mounts) {
            let path = Path::new(&dev);
            // FUSE filesystems (fuseblk, fuse.*) can hang in statvfs when
            // their daemon stops, even when backed by a block device.
            if !path.starts_with("/dev") || fs.starts_with("fuse") {
                continue;
            }
            let Some(device) = device_name(&self.root, path) else {
                continue;
            };
            if chosen
                .get(&device)
                .is_some_and(|(m, _)| m.len() <= mount_point.len())
            {
                continue;
            }
            chosen.insert(device, (mount_point, fs));
        }
        let missing = if counters.is_none() {
            "diskstats not readable"
        } else {
            "device counters not reported"
        };
        let mut out: Vec<DiskStats> = Vec::new();
        for (device, (mount_point, file_system)) in chosen {
            let Some((total_bytes, available_bytes)) = (self.space)(Path::new(&mount_point)) else {
                continue;
            };
            let (read_bps, write_bps) = rates
                .get(&device)
                .cloned()
                .unwrap_or_else(|| (Reading::unavailable(missing), Reading::unavailable(missing)));
            out.push(DiskStats {
                mount_point,
                device,
                file_system,
                total_bytes,
                available_bytes,
                read_bps,
                write_bps,
            });
        }
        out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
        Reading::Available(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn queried(mounts: &[u8]) -> Vec<PathBuf> {
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree.path().join("proc/self")).unwrap();
        std::fs::write(tree.path().join("proc/self/mounts"), mounts).unwrap();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = asked.clone();
        let mut d = Disks::with_space(tree.path().into(), move |p: &Path| {
            log.lock().unwrap().push(p.to_path_buf());
            Some((1000, 250))
        });
        d.sample();
        asked.lock().unwrap().clone()
    }

    #[test]
    fn fuse_filesystems_on_block_devices_are_not_queried() {
        let asked = queried(b"/dev/sdb1 /mnt/win fuseblk rw 0 0\n/dev/sda1 / ext4 rw 0 0\n");
        assert_eq!(asked, vec![PathBuf::from("/")]);
    }

    #[test]
    fn a_mount_hidden_under_a_later_mount_is_not_queried() {
        let asked = queried(
            b"/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /data ext4 rw 0 0\nserver:/x /data nfs4 rw 0 0\n",
        );
        assert_eq!(asked, vec![PathBuf::from("/")]);
    }

    #[test]
    fn only_ascii_whitespace_separates_mount_fields() {
        let asked = queried("/dev/sda1 /mnt/a\u{a0}b ext4 rw 0 0\n".as_bytes());
        assert_eq!(asked, vec![PathBuf::from("/mnt/a\u{a0}b")]);
    }

    #[test]
    fn one_undecodable_record_does_not_lose_the_others() {
        let asked = queried(b"/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /mnt/\xff ext4 rw 0 0\n");
        assert_eq!(asked, vec![PathBuf::from("/")]);
    }

    #[test]
    fn only_block_device_mounts_are_queried_for_space() {
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree.path().join("proc/self")).unwrap();
        std::fs::create_dir_all(tree.path().join("dev")).unwrap();
        std::fs::write(tree.path().join("dev/nvme0n1p2"), "").unwrap();
        std::fs::write(
            tree.path().join("proc/self/mounts"),
            "/dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0\n\
             /dev/nvme0n1p2 /home btrfs rw,subvol=/@home 0 0\n\
             proc /proc proc rw 0 0\n\
             alice@host:/srv /home/alice/remote fuse.sshfs rw 0 0\n\
             server:/export /mnt/nfs nfs4 rw 0 0\n\
             /dev/sdb1 /mnt/My\\040Drive ext4 rw 0 0\n",
        )
        .unwrap();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = asked.clone();
        let mut d = Disks::with_space(tree.path().into(), move |p: &Path| {
            log.lock().unwrap().push(p.to_path_buf());
            Some((1000, 250))
        });
        let disks = d.sample();
        let disks = disks.value().unwrap();
        let asked = asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![PathBuf::from("/"), PathBuf::from("/mnt/My Drive")],
            "statvfs only on deduplicated block-device mounts"
        );
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].mount_point, "/");
        assert_eq!(disks[0].total_bytes, 1000);
        assert_eq!(disks[0].available_bytes, 250);
        assert_eq!(disks[1].mount_point, "/mnt/My Drive");
    }
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
