//! Utilify backend. No UI and no gpui dependency.
//!
//! - [`alerts`]: the notification rules, a pure engine fed once per tick.
//! - [`awake`]: keep-awake sessions (power assertions), like Amphetamine.
//! - [`power`]: the power source (AC or battery, charge, Low Power Mode).
//! - [`net`]: per-interface byte counters and link speed, and the
//!   [`net::NetMeter`] that turns two readings into rates.
//! - [`disk`]: capacity and free space of the startup volume.
//! - [`disk_history`]: storage use over time, persisted across launches.
//! - [`fans`]: fan presets and curves, and control through the root helper.
//! - [`monitor`]: one tick of sampling, plus the rolling history.
//! - [`icon`]: the menu-bar graph, built as SVG and rasterized with resvg.
//! - [`ipc`]: the socket `utilifyctl` uses to read and drive the running app.
//! - [`format`]: human-readable sizes and rates.
//! - [`health`]: periodic self-checks of the data directory, tools and files.
//! - [`sensors`]: named temperature sensors (Apple Silicon).
//! - [`settings`]: the persisted preferences.
//! - [`speedtest`]: on-demand speed test via `networkQuality`.
//!
//! Persisted files that exist but don't parse are moved aside rather than
//! overwritten (see [`take_recovered_files`]).

pub mod alerts;
pub mod awake;
pub mod disk;
pub mod disk_history;
pub mod fans;
pub mod format;
pub mod health;
pub mod history;
pub mod icon;
pub mod ipc;
pub mod monitor;
pub mod net;
pub mod power;
pub mod sensors;
pub mod settings;
pub mod speedtest;

/// Names the per-user data directory.
pub const APP_IDENTIFIER: &str = "dev.utilify.app";
pub const PRODUCT_NAME: &str = "Utilify";

/// Write `bytes` to `path` atomically, creating its directory: readers see
/// the old file or the new one, never a partial write. Each save gets its own
/// temporary file, so overlapping saves of one path can't corrupt it.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => std::path::Path::new("."),
    };
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Files moved aside by [`load_json`] since the last
/// [`take_recovered_files`].
static RECOVERED: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// The corrupt files moved aside since the last call (their new paths), so
/// the app can tell the user once. Empties the list.
pub fn take_recovered_files() -> Vec<std::path::PathBuf> {
    std::mem::take(&mut *RECOVERED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Whether `path` is waiting in [`take_recovered_files`], without taking
/// it (tests run in parallel and share the list).
#[cfg(test)]
pub(crate) fn recovered_contains(path: &std::path::Path) -> bool {
    RECOVERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|p| p == path)
}

/// Where a corrupt `path` is moved: `settings.json` becomes
/// `settings.corrupt-<unix secs>.json` in the same directory.
pub fn corrupt_path(path: &std::path::Path, unix_secs: u64) -> std::path::PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "file".into(), |s| s.to_string_lossy());
    let ext = path
        .extension()
        .map_or_else(|| "json".into(), |s| s.to_string_lossy());
    path.with_file_name(format!("{stem}.corrupt-{unix_secs}.{ext}"))
}

/// Move a file that failed to parse out of the way, so the next save doesn't
/// overwrite what the user had, and remember it for
/// [`take_recovered_files`]. Never replaces an earlier corrupt copy. The new
/// path, or `None` if the move failed.
pub fn quarantine(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let secs = disk_history::unix_now();
    let mut to = corrupt_path(path, secs);
    let mut n = 1;
    while to.exists() {
        to = corrupt_path(path, secs).with_extension(format!("{n}.json"));
        n += 1;
    }
    match std::fs::rename(path, &to) {
        Ok(()) => {
            log::warn!(
                "{} didn't parse; moved it to {}",
                path.display(),
                to.display()
            );
            RECOVERED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(to.clone());
            Some(to)
        }
        Err(e) => {
            log::warn!("Moving aside unreadable {} failed: {e}", path.display());
            None
        }
    }
}

/// Read and parse a JSON file. `None` when it's missing or unreadable, or
/// when it doesn't parse, in which case it's first moved aside with
/// [`quarantine`]. `what` names it in the log.
pub(crate) fn load_json<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
    what: &str,
) -> Option<T> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Reading {what} at {} failed: {e}", path.display());
            }
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("Ignoring unreadable {what} at {}: {e}", path.display());
            quarantine(path);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_files_are_moved_aside_and_reported() {
        assert_eq!(
            corrupt_path(std::path::Path::new("/d/settings.json"), 42),
            std::path::Path::new("/d/settings.corrupt-42.json")
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        assert_eq!(load_json::<u32>(&path, "x"), None, "missing");
        std::fs::write(&path, "7").unwrap();
        assert_eq!(load_json::<u32>(&path, "x"), Some(7));
        std::fs::write(&path, "oops").unwrap();
        assert_eq!(load_json::<u32>(&path, "x"), None);
        assert!(!path.exists());
        // A second corruption in the same second keeps the first copy.
        std::fs::write(&path, "again").unwrap();
        assert_eq!(load_json::<u32>(&path, "x"), None);
        let moved: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(moved.len(), 2);
        // Other tests quarantine into the same list at the same time and
        // check their own entries, so don't drain it here.
        assert!(moved.iter().all(|m| recovered_contains(m)));
    }

    #[test]
    fn overlapping_atomic_writes_leave_one_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("data.json");
        let writers: Vec<_> = (0..8u8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        write_atomic(&path, &vec![b'a' + i; 64 * 1024]).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 64 * 1024);
        assert!(bytes.iter().all(|&b| b == bytes[0]));
        // No temporary files are left behind.
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }
}
