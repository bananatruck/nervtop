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

/// Decodes the octal escapes the kernel uses for spaces, tabs, newlines and
/// backslashes in mount table paths. The result stays as bytes: a mount point
/// need not be valid UTF-8.
fn unescape_mount(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut i = 0;
    while i < field.len() {
        if field[i] == b'\\'
            && i + 3 < field.len()
            && field[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let v = (field[i + 1] - b'0') as u32 * 64
                + (field[i + 2] - b'0') as u32 * 8
                + (field[i + 3] - b'0') as u32;
            if let Ok(b) = u8::try_from(v) {
                out.push(b);
                i += 4;
                continue;
            }
        }
        out.push(field[i]);
        i += 1;
    }
    out
}

/// A mount point for display: valid UTF-8 as is, other bytes as `\xNN`.
fn display_path(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        for b in chunk.invalid() {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out
}

fn as_path(bytes: &[u8]) -> &Path {
    use std::os::unix::ffi::OsStrExt;
    Path::new(std::ffi::OsStr::from_bytes(bytes))
}

/// One record of `/proc/self/mountinfo`, as proc_pid_mountinfo(5) describes:
/// `ID PARENT MAJ:MIN ROOT MOUNT-POINT OPTIONS [OPTIONAL...] - TYPE SOURCE SUPER-OPTIONS`.
#[derive(Debug, Clone, PartialEq)]
struct Mount {
    id: u64,
    parent: u64,
    point: Vec<u8>,
    fs: String,
    source: Vec<u8>,
}

/// Parses a mountinfo table. Fields are separated by ASCII spaces and tabs
/// only, so a no-break space inside a path survives; a malformed record is
/// skipped on its own.
fn parse_mountinfo(table: &[u8]) -> Vec<Mount> {
    table
        .split(|b| *b == b'\n')
        .filter_map(|line| {
            let f: Vec<&[u8]> = line
                .split(|b| *b == b' ' || *b == b'\t')
                .filter(|x| !x.is_empty())
                .collect();
            let number = |x: &[u8]| std::str::from_utf8(x).ok()?.parse::<u64>().ok();
            // The optional fields end at a lone "-", after the six fixed fields.
            let dash = 6 + f.get(6..)?.iter().position(|x| *x == b"-")?;
            Some(Mount {
                id: number(f.first()?)?,
                parent: number(f.get(1)?)?,
                point: unescape_mount(f.get(4)?),
                fs: String::from_utf8(unescape_mount(f.get(dash + 1)?)).ok()?,
                source: unescape_mount(f.get(dash + 2)?),
            })
        })
        .collect()
}

/// The mounts that path lookup reaches at their own mount point, in table
/// order, decided from the mount tree rather than from paths alone.
///
/// `reach(i)` holds when looking up mount i's mount point lands on the stack
/// of mounts at that path that contains i:
/// - a mount on top of its parent at the same path is reached when the
///   parent is;
/// - otherwise the mount sits on a directory inside its parent, and is
///   reached only when no other mount on that parent (by parent ID; the
///   parent itself excluded, since a root may name itself as its parent)
///   covers one of its ancestor directories, no later mount on that parent
///   has the same path, and the parent is visible. A parent whose record is
///   absent (outside the process root, as in a chroot or container, or the
///   root's own parent) cannot be judged, so it counts as visible: such a
///   mount is shown unless a sibling covers it.
///
/// A mount is visible when it is reached and nothing sits on it (no child at
/// the same path). So when NFS covers an ext4 `/data`, an ext4 mount at
/// `/data/local` that sat on the old `/data` is hidden: querying its path
/// would reach NFS. A parent cycle in a malformed table hides its mounts.
fn visible(mounts: &[Mount]) -> Vec<&Mount> {
    let index: std::collections::HashMap<u64, usize> =
        mounts.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
    let parent_of = |i: usize| index.get(&mounts[i].parent).copied().filter(|&p| p != i);
    let covered = |i: usize| {
        mounts
            .iter()
            .any(|c| c.id != mounts[i].id && c.parent == mounts[i].id && c.point == mounts[i].point)
    };
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        New,
        Visiting,
        Done(bool),
    }
    fn reach(
        i: usize,
        mounts: &[Mount],
        parent_of: &dyn Fn(usize) -> Option<usize>,
        covered: &dyn Fn(usize) -> bool,
        state: &mut [State],
    ) -> bool {
        match state[i] {
            State::Done(r) => return r,
            State::Visiting => return false,
            State::New => {}
        }
        state[i] = State::Visiting;
        let m = &mounts[i];
        let parent = parent_of(i);
        let r = match parent {
            Some(p) if mounts[p].point == m.point => reach(p, mounts, parent_of, covered, state),
            _ => {
                // Siblings are the other mounts recorded on the same parent
                // ID, whether or not that parent's own record is in the table;
                // the parent itself is not one. A root that names itself as
                // its parent has none: mounts naming it are its children.
                let point = as_path(&m.point);
                let hidden_by_sibling = m.parent != m.id
                    && mounts.iter().enumerate().any(|(j, s)| {
                        j != i
                            && Some(j) != parent
                            && s.parent == m.parent
                            && if s.point == m.point {
                                j > i
                            } else {
                                point.starts_with(as_path(&s.point))
                            }
                    });
                // With the parent's record absent, whether the parent is
                // itself reached cannot be decided, so it is taken as reached:
                // an undecidable mount is shown rather than hidden.
                !hidden_by_sibling
                    && parent
                        .is_none_or(|p| !covered(p) && reach(p, mounts, parent_of, covered, state))
            }
        };
        state[i] = State::Done(r);
        r
    }
    let mut state = vec![State::New; mounts.len()];
    (0..mounts.len())
        .filter(|&i| reach(i, mounts, &parent_of, &covered, &mut state) && !covered(i))
        .map(|i| &mounts[i])
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
        let Ok(table) = std::fs::read(self.root.join("proc/self/mountinfo")) else {
            return Reading::unavailable("mount table not readable");
        };
        let counters = read(self.root.join("proc/diskstats"));
        let rates = self
            .rates
            .sample(counters.as_deref().map(parse_diskstats).unwrap_or_default());
        // Filter before asking for space: statvfs on a FUSE or network mount
        // whose server has gone away can block indefinitely, so only visible
        // block-device mounts are ever queried.
        let mounts = parse_mountinfo(&table);
        let mut chosen: BTreeMap<String, Vec<&Mount>> = BTreeMap::new();
        for mount in visible(&mounts) {
            let path = as_path(&mount.source);
            // FUSE filesystems (fuseblk, fuse.*) can hang in statvfs when
            // their daemon stops, even when backed by a block device.
            if !path.starts_with("/dev") || mount.fs.starts_with("fuse") {
                continue;
            }
            let Some(device) = device_name(&self.root, path) else {
                continue;
            };
            chosen.entry(device).or_default().push(mount);
        }
        let missing = if counters.is_none() {
            "diskstats not readable"
        } else {
            "device counters not reported"
        };
        let mut out: Vec<DiskStats> = Vec::new();
        for (device, mut candidates) in chosen {
            // Btrfs subvolumes share filesystem capacity; keep one pool per device.
            // Linux btrfs_statfs uses fs_info for capacity, not the subvolume:
            // https://github.com/torvalds/linux/blob/v6.12/fs/btrfs/super.c#L1631-L1726
            // The shortest mount point is tried first; a longer one is the
            // fallback when that path cannot be queried.
            candidates.sort_by_key(|m| m.point.len());
            let Some((mount, (total_bytes, available_bytes))) = candidates
                .into_iter()
                .find_map(|m| (self.space)(as_path(&m.point)).map(|space| (m, space)))
            else {
                continue;
            };
            let (read_bps, write_bps) = rates
                .get(&device)
                .cloned()
                .unwrap_or_else(|| (Reading::unavailable(missing), Reading::unavailable(missing)));
            out.push(DiskStats {
                mount_point: display_path(&mount.point),
                device,
                file_system: mount.fs.clone(),
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
    use std::os::unix::ffi::OsStrExt;

    /// Samples a tree whose mountinfo is `table`, answering statvfs with
    /// `space`, and returns the paths queried and the disks reported.
    fn sampled(
        table: &[u8],
        space: impl Fn(&Path) -> Option<(u64, u64)> + Send + 'static,
    ) -> (Vec<PathBuf>, Vec<DiskStats>) {
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree.path().join("proc/self")).unwrap();
        std::fs::write(tree.path().join("proc/self/mountinfo"), table).unwrap();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = asked.clone();
        let mut d = Disks::with_space(tree.path().into(), move |p: &Path| {
            log.lock().unwrap().push(p.to_path_buf());
            space(p)
        });
        let disks = d.sample().value().cloned().unwrap();
        let asked = asked.lock().unwrap().clone();
        (asked, disks)
    }

    fn queried(table: &[u8]) -> Vec<PathBuf> {
        sampled(table, |_| Some((1000, 250))).0
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn fuse_filesystems_on_block_devices_are_not_queried() {
        let asked = queried(
            b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:17 / /mnt/win rw - fuseblk /dev/sdb1 rw\n",
        );
        assert_eq!(asked, paths(&["/"]));
    }

    #[test]
    fn a_mount_hidden_under_a_later_mount_is_not_queried() {
        let asked = queried(
            b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
              2 1 8:17 / /data rw - ext4 /dev/sdb1 rw\n\
              3 2 0:50 / /data rw - nfs4 server:/x rw\n",
        );
        assert_eq!(asked, paths(&["/"]));
    }

    /// NFS mounted over `/data` hides the ext4 mount below it and every mount
    /// that sat on that ext4 mount, such as `/data/local`; a local mount made
    /// on top of the NFS mount afterwards is visible.
    #[test]
    fn mounts_under_an_overmounted_ancestor_are_not_queried() {
        let asked = queried(
            b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
              2 1 8:17 / /data rw - ext4 /dev/sdb1 rw\n\
              3 2 8:33 / /data/local rw - ext4 /dev/sdc1 rw\n\
              4 2 0:50 / /data rw - nfs4 server:/x rw\n\
              5 4 8:49 / /data/later rw - ext4 /dev/sdd1 rw\n",
        );
        assert_eq!(asked, paths(&["/", "/data/later"]));
    }

    /// In a chroot, NFS on /data and an ext4 /data/local below it share a
    /// parent whose record is absent; statvfs must not reach NFS through
    /// /data/local.
    #[test]
    fn a_covered_mount_on_an_absent_parent_is_not_queried() {
        let asked = queried(
            b"3 1 8:3 / /data/local rw - ext4 /dev/sdc1 rw\n\
              4 1 0:50 / /data rw - nfs4 server:/x rw\n\
              5 1 8:17 / /srv rw - ext4 /dev/sdb1 rw\n",
        );
        assert_eq!(asked, paths(&["/srv"]));
    }

    /// Which mounts are visible, by mount and parent IDs, for each way one
    /// mount can hide another.
    #[test]
    fn visibility_follows_the_mount_tree() {
        for (table, expected) in [
            // A stack of three mounts at one path: only the top is visible.
            (
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:2 / /a rw - ext4 /dev/sdb rw\n3 2 8:3 / /a rw - ext4 /dev/sdc rw\n4 3 8:4 / /a rw - ext4 /dev/sdd rw\n",
                &["/", "/a"][..],
            ),
            // A child of a mount in the middle of a stack is hidden.
            (
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:2 / /a rw - ext4 /dev/sdb rw\n3 2 8:3 / /a rw - ext4 /dev/sdc rw\n5 3 8:5 / /a/b rw - ext4 /dev/sde rw\n4 3 8:4 / /a rw - ext4 /dev/sdd rw\n",
                &["/", "/a"],
            ),
            // A sibling mounted on an ancestor directory hides a deeper mount
            // made on the same parent earlier.
            (
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n3 1 8:3 / /data/local rw - ext4 /dev/sdc rw\n2 1 8:2 / /data rw - ext4 /dev/sdb rw\n",
                &["/", "/data"],
            ),
            // A path that only shares a prefix is not an ancestor.
            (
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:2 / /data rw - ext4 /dev/sdb rw\n3 1 8:3 / /database rw - ext4 /dev/sdc rw\n",
                &["/", "/data", "/database"],
            ),
            // An overmounted root hides everything mounted on the old root.
            (
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:2 / /home rw - ext4 /dev/sdb rw\n3 1 8:3 / / rw - ext4 /dev/sdc rw\n",
                &["/"],
            ),
            // Inside a chroot or container the parent record can be absent
            // (the kernel skips mounts outside the process root). Siblings on
            // that absent parent still cover each other: NFS on /data hides an
            // ext4 /data/local mounted on the same parent before it.
            (
                "3 1 8:3 / /data/local rw - ext4 /dev/sdc rw\n4 1 0:50 / /data rw - nfs4 server:/x rw\n",
                &["/data"],
            ),
            // Mounts on an absent parent that no sibling covers stay visible.
            (
                "3 1 0:5 / /proc rw - proc proc rw\n4 1 8:4 / /data rw - ext4 /dev/sdd rw\n5 1 8:5 / /data2 rw - ext4 /dev/sde rw\n",
                &["/proc", "/data", "/data2"],
            ),
            // The namespace root may name itself as its parent; it is not a
            // sibling of its own children.
            (
                "1 1 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:2 / /data rw - ext4 /dev/sdb rw\n3 1 8:3 / /boot rw - ext4 /dev/sdc rw\n",
                &["/", "/data", "/boot"],
            ),
            // A self-parented root overmounted at / still hides what sat on it.
            (
                "1 1 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:2 / /home rw - ext4 /dev/sdb rw\n3 1 8:3 / / rw - ext4 /dev/sdc rw\n",
                &["/"],
            ),
            // A parent cycle in a malformed table hides its mounts.
            (
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n7 8 8:7 / /x rw - ext4 /dev/sdx rw\n8 7 8:8 / /y rw - ext4 /dev/sdy rw\n",
                &["/"],
            ),
        ] {
            let mounts = parse_mountinfo(table.as_bytes());
            let seen: Vec<&str> = visible(&mounts)
                .iter()
                .map(|m| std::str::from_utf8(&m.point).unwrap())
                .collect();
            assert_eq!(seen, expected, "{table}");
        }
    }

    #[test]
    fn mountinfo_fields_follow_the_separator_after_optional_fields() {
        let mounts = parse_mountinfo(
            b"36 35 98:0 /mnt1 /mnt/My\\040Drive rw,noatime master:1 shared:2 - ext3 /dev/root rw,errors=continue\n\
              37 35 98:0 / /broken rw no-separator ext3 /dev/x rw\n",
        );
        assert_eq!(
            mounts,
            vec![Mount {
                id: 36,
                parent: 35,
                point: b"/mnt/My Drive".to_vec(),
                fs: "ext3".into(),
                source: b"/dev/root".to_vec(),
            }]
        );
    }

    #[test]
    fn only_ascii_whitespace_separates_mount_fields() {
        let asked = queried("1 0 8:1 / /mnt/a\u{a0}b rw - ext4 /dev/sda1 rw\n".as_bytes());
        assert_eq!(asked, paths(&["/mnt/a\u{a0}b"]));
    }

    /// A mount point that is not valid UTF-8 is queried by its bytes and
    /// shown with the undecodable bytes escaped, so the disk is not lost.
    #[test]
    fn a_mount_point_that_is_not_utf8_is_queried_and_shown_escaped() {
        let (asked, disks) = sampled(
            b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:17 / /mnt/\xff rw - ext4 /dev/sdb1 rw\n",
            |_| Some((1000, 250)),
        );
        assert_eq!(
            asked,
            vec![
                PathBuf::from("/"),
                PathBuf::from(std::ffi::OsStr::from_bytes(b"/mnt/\xff"))
            ]
        );
        let shown: Vec<&str> = disks.iter().map(|d| d.mount_point.as_str()).collect();
        assert_eq!(shown, ["/", "/mnt/\\xff"]);
    }

    /// When the preferred (shortest) mount point of a device cannot be
    /// queried, a longer one for the same device is tried, and the device
    /// still gets one row.
    #[test]
    fn a_device_falls_back_to_another_mount_point() {
        let (asked, disks) = sampled(
            b"1 0 0:30 /@ / rw - btrfs /dev/nvme0n1p2 rw\n\
              2 1 0:30 /@home /home rw - btrfs /dev/nvme0n1p2 rw\n",
            |p| (p != Path::new("/")).then_some((1000, 250)),
        );
        assert_eq!(asked, paths(&["/", "/home"]));
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].mount_point, "/home");
        assert_eq!(disks[0].device, "nvme0n1p2");
    }

    #[test]
    fn only_block_device_mounts_are_queried_for_space() {
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree.path().join("proc/self")).unwrap();
        std::fs::create_dir_all(tree.path().join("dev")).unwrap();
        std::fs::write(tree.path().join("dev/nvme0n1p2"), "").unwrap();
        std::fs::write(
            tree.path().join("proc/self/mountinfo"),
            "1 0 0:30 /@ / rw - btrfs /dev/nvme0n1p2 rw,subvol=/@\n\
             2 1 0:30 /@home /home rw - btrfs /dev/nvme0n1p2 rw,subvol=/@home\n\
             3 1 0:5 / /proc rw - proc proc rw\n\
             4 2 0:60 / /home/alice/remote rw - fuse.sshfs alice@host:/srv rw\n\
             5 1 0:61 / /mnt/nfs rw - nfs4 server:/export rw\n\
             6 1 8:17 / /mnt/My\\040Drive rw - ext4 /dev/sdb1 rw\n",
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
