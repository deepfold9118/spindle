//! Small OS helpers.

use std::path::Path;

/// Open a folder in the system file manager.
pub fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(windows)]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}
