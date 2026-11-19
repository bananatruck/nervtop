use super::Source;
use crate::model::{MemoryStats, Reading};
use std::{collections::BTreeMap, path::PathBuf};

pub struct Memory {
    root: PathBuf,
}

impl Memory {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new("/".into())
    }
}

impl Source for Memory {
    type Output = MemoryStats;

    fn sample(&mut self) -> Reading<MemoryStats> {
        let Ok(text) = std::fs::read_to_string(self.root.join("proc/meminfo")) else {
            return Reading::unavailable("meminfo not readable");
        };
        match parse_meminfo(&text) {
            Some(stats) => Reading::Available(stats),
            None => Reading::unavailable("invalid meminfo"),
        }
    }
}

fn parse_meminfo(text: &str) -> Option<MemoryStats> {
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !matches!(
            key,
            "MemTotal"
                | "MemAvailable"
                | "MemFree"
                | "Buffers"
                | "Cached"
                | "SReclaimable"
                | "Shmem"
                | "SwapTotal"
                | "SwapFree"
        ) {
            continue;
        }
        let mut parts = value.split_ascii_whitespace();
        // The kernel labels these values kB, but the unit is 1024 bytes.
        let bytes = parts.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
        if parts.next() != Some("kB")
            || parts.next().is_some()
            || fields.insert(key, bytes).is_some()
        {
            return None;
        }
    }
    let total_bytes = *fields.get("MemTotal")?;
    let available_bytes = match fields.get("MemAvailable") {
        Some(bytes) => *bytes,
        // Match sysinfo's fallback for kernels before Linux 3.14.
        None => fields
            .get("MemFree")?
            .saturating_add(*fields.get("Buffers")?)
            .saturating_add(*fields.get("Cached")?)
            .saturating_add(*fields.get("SReclaimable")?)
            .saturating_sub(*fields.get("Shmem")?),
    };
    let swap_total_bytes = *fields.get("SwapTotal")?;
    let swap_used_bytes = swap_total_bytes.checked_sub(*fields.get("SwapFree")?)?;
    if total_bytes == 0 || available_bytes > total_bytes {
        return None;
    }
    Some(MemoryStats {
        total_bytes,
        available_bytes,
        swap_total_bytes,
        swap_used_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const MEMINFO: &str =
        "MemTotal: 1000 kB\nMemAvailable: 400 kB\nSwapTotal: 200 kB\nSwapFree: 50 kB\n";

    #[test]
    fn failed_memory_refresh_is_unavailable_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("proc")).unwrap();
        let path = dir.path().join("proc/meminfo");
        fs::write(&path, MEMINFO).unwrap();
        let mut memory = Memory::new(dir.path().into());
        assert!(memory.sample().is_available());
        fs::remove_file(&path).unwrap();
        assert!(
            !memory.sample().is_available(),
            "must not publish the last successful sample"
        );
        fs::write(&path, MEMINFO.replace("400", "600")).unwrap();
        assert_eq!(memory.sample().value().unwrap().available_bytes, 600 * 1024);
    }
    #[test]
    fn parses_current_values_and_rejects_incomplete_or_invalid_samples() {
        let stats = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(stats.total_bytes, 1000 * 1024);
        assert_eq!(stats.available_bytes, 400 * 1024);
        assert_eq!(stats.swap_total_bytes, 200 * 1024);
        assert_eq!(stats.swap_used_bytes, 150 * 1024);
        for text in [
            String::new(),
            MEMINFO.replace("1000", "0"),
            MEMINFO.replace("400", "1001"),
            MEMINFO.replace("50 kB", "201 kB"),
            MEMINFO.replace("400 kB", "bad kB"),
            MEMINFO.replace("400 kB", "400 MB"),
            MEMINFO.replace("SwapFree: 50 kB\n", ""),
            MEMINFO.replace("1000", "18446744073709551615"),
        ] {
            assert!(parse_meminfo(&text).is_none(), "{text}");
        }
        let no_swap =
            parse_meminfo(&MEMINFO.replace("200 kB", "0 kB").replace("50 kB", "0 kB")).unwrap();
        assert_eq!(no_swap.swap_used_bytes, 0);
        let old_kernel = MEMINFO.replace(
            "MemAvailable: 400 kB",
            "MemFree: 100 kB\nBuffers: 20 kB\nCached: 300 kB\nSReclaimable: 40 kB\nShmem: 60 kB",
        );
        assert_eq!(
            parse_meminfo(&old_kernel).unwrap().available_bytes,
            400 * 1024
        );
    }
}
