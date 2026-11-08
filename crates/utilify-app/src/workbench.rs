//! Starting the workbench. It's a separate program, `utilify-workbench`
//! (in `workbench/` at the top of the repository), that sits next to the
//! app's executable: `Utilify.app/Contents/MacOS`, or `target/<profile>` when
//! run from the source tree. It's optional. It needs the utilify-codecs
//! checkout to build, so `install.sh` leaves it out when that's missing, and
//! then the menu item, the dashboard button, ⌘B and `utilifyctl workbench
//! open` are turned off.
//!
//! A running workbench holds an exclusive lock on `workbench.lock`, next to
//! the settings, with its process id in it. That's how the app tells whether
//! it's open, even one it didn't start, and how a second copy finds the
//! first and brings it forward instead of opening another window.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The workbench's file name, next to the app's executable.
pub const BINARY: &str = "utilify-workbench";

/// Why it can't be opened, for the dashboard's tooltip and `utilifyctl`.
pub const UNAVAILABLE: &str = "This copy of Utilify was built without the workbench. \
     Clone utilify-codecs next to the source and run install.sh again to add it.";

/// Where the workbench is, if this build has one. Looked up once: an
/// install replaces the whole app, so it can't appear while this one runs.
pub fn path() -> Option<&'static PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let path = exe.parent()?.join(BINARY);
        let found = path.is_file();
        log::info!(
            "Workbench {}: {}",
            if found { "found" } else { "not built" },
            path.display()
        );
        found.then_some(path)
    })
    .as_ref()
}

pub fn is_available() -> bool {
    path().is_some()
}

/// Whether a workbench is running.
pub fn is_open() -> bool {
    running_pid().is_some()
}

/// Bring the running workbench forward, or start one.
pub fn open() -> Result<(), String> {
    let Some(exe) = path() else {
        return Err(UNAVAILABLE.into());
    };
    if let Some(pid) = running_pid()
        && activate(pid)
    {
        return Ok(());
    }
    let mut child = Command::new(exe)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start the workbench: {e}"))?;
    // Collect its exit status when it closes, so it doesn't linger as a zombie.
    std::thread::Builder::new()
        .name("workbench-wait".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .map_err(|e| format!("Couldn't start the workbench: {e}"))?;
    Ok(())
}

fn lock_path() -> Option<PathBuf> {
    utilify_core::settings::default_path().map(|p| p.with_file_name("workbench.lock"))
}

/// The process id of the running workbench, from its lock file.
fn running_pid() -> Option<i32> {
    let mut file = File::open(lock_path()?).ok()?;
    match file.try_lock() {
        // Nobody holds it: no workbench. Dropping the file lets go of it.
        Ok(()) => None,
        Err(TryLockError::WouldBlock) => {
            let mut text = String::new();
            file.read_to_string(&mut text).ok()?;
            text.trim().parse().ok()
        }
        Err(TryLockError::Error(e)) => {
            log::warn!("Checking for a running workbench failed: {e}");
            None
        }
    }
}

/// Held by a running workbench for as long as it runs.
pub struct Claim {
    _file: Option<File>,
}

/// For the workbench itself: take the lock, or, if another workbench holds
/// it, bring that one forward and get `None` back (and quit). A lock that
/// can't be taken for any other reason gives a claim on nothing, so the
/// window still opens.
pub fn claim() -> Option<Claim> {
    let Some(path) = lock_path() else {
        return Some(Claim { _file: None });
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            log::warn!("Couldn't open {}: {e}", path.display());
            return Some(Claim { _file: None });
        }
    };
    match file.try_lock() {
        Ok(()) => {
            let mut file = file;
            let written = file
                .set_len(0)
                .and_then(|()| file.rewind())
                .and_then(|()| write!(file, "{}", std::process::id()));
            if let Err(e) = written {
                log::warn!("Couldn't write {}: {e}", path.display());
            }
            Some(Claim { _file: Some(file) })
        }
        Err(TryLockError::WouldBlock) => {
            if let Some(pid) = running_pid() {
                activate(pid);
            }
            None
        }
        Err(TryLockError::Error(e)) => {
            log::warn!("Couldn't lock {}: {e}", path.display());
            Some(Claim { _file: Some(file) })
        }
    }
}

/// Bring the process forward, with all its windows.
#[cfg(target_os = "macos")]
fn activate(pid: i32) -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return false;
    };
    // Without "ignoring other apps" the request is ignored before macOS 14
    // when we aren't frontmost (a click in the menu bar doesn't make us so).
    // From 14 on it has no effect and the system decides.
    #[allow(deprecated)]
    let options = NSApplicationActivationOptions::ActivateAllWindows
        | NSApplicationActivationOptions::ActivateIgnoringOtherApps;
    app.activateWithOptions(options)
}

#[cfg(not(target_os = "macos"))]
fn activate(_pid: i32) -> bool {
    false
}
