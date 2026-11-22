//! Desktop integration: starting Spindle when the user signs in, and opening `.blend`
//! files with Spindle (which picks a build for each file, see `blendfile`).
//!
//! - Windows: `HKCU\...\CurrentVersion\Run`, and an "Open with" registration under
//!   `HKCU\Software\Classes`. Windows doesn't let programs make themselves the default
//!   app; `open_default_apps()` opens the Settings page where the user picks Spindle.
//! - Linux: `~/.config/autostart/spindle.desktop`, and `~/.local/share/applications/
//!   spindle.desktop` with the `application/x-blender` MIME type (also made the default).
//! - macOS: a LaunchAgent for signing in. File associations need an app bundle, which
//!   Spindle doesn't ship yet, so `set_blend_association` reports that.

use anyhow::Result;
#[cfg(not(windows))]
use anyhow::Context;
use std::path::{Path, PathBuf};

/// Argument Spindle is started with at sign-in: straight to the tray, no window.
pub const TRAY_ARG: &str = "--tray";

/// The executable registered to start at sign-in, if any.
pub fn autostart() -> Option<PathBuf> {
    imp::autostart()
}

/// Start (or stop starting) `app_exe --tray` when the user signs in.
pub fn set_autostart(on: bool, app_exe: &Path) -> Result<()> {
    imp::set_autostart(on, app_exe)
}

/// The executable registered to open `.blend` files, if any.
pub fn blend_association() -> Option<PathBuf> {
    imp::blend_association()
}

/// Register (or unregister) `app_exe` for opening `.blend` files.
pub fn set_blend_association(on: bool, app_exe: &Path) -> Result<()> {
    imp::set_blend_association(on, app_exe)
}

/// Where the user chooses the default app for `.blend` files (Windows Settings).
/// Returns false when the platform has no such page.
pub fn open_default_apps() -> bool {
    imp::open_default_apps()
}

/// Re-point existing registrations at `app_exe` (after Spindle moved or was updated).
pub fn refresh(app_exe: &Path) {
    if autostart().is_some_and(|p| p != app_exe) {
        let _ = set_autostart(true, app_exe);
    }
    if blend_association().is_some_and(|p| p != app_exe) {
        let _ = set_blend_association(true, app_exe);
    }
}

/// `"C:\x\spindle_app.exe" --tray` -> `C:\x\spindle_app.exe`
#[cfg_attr(not(windows), allow(dead_code))]
fn exe_of_command(cmd: &str) -> Option<PathBuf> {
    let cmd = cmd.trim();
    let exe = match cmd.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => cmd.split(' ').next()?,
    };
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

#[cfg(windows)]
mod imp {
    use super::{TRAY_ARG, exe_of_command};
    use anyhow::{Context, Result};
    use std::path::{Path, PathBuf};
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE: &str = "Spindle";
    const PROGID: &str = "Spindle.blend";
    const CLASSES: &str = r"Software\Classes";

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    pub fn autostart() -> Option<PathBuf> {
        let cmd: String = hkcu().open_subkey_with_flags(RUN, KEY_READ).ok()?.get_value(RUN_VALUE).ok()?;
        exe_of_command(&cmd)
    }

    pub fn set_autostart(on: bool, app_exe: &Path) -> Result<()> {
        let (run, _) = hkcu().create_subkey(RUN).context("opening the Run key")?;
        if on {
            run.set_value(RUN_VALUE, &format!("\"{}\" {TRAY_ARG}", app_exe.display()))
                .context("writing the Run entry")?;
        } else if run.get_raw_value(RUN_VALUE).is_ok() {
            run.delete_value(RUN_VALUE).context("removing the Run entry")?;
        }
        Ok(())
    }

    fn app_key(app_exe: &Path) -> String {
        let file = app_exe.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or("spindle_app.exe".into());
        format!(r"{CLASSES}\Applications\{file}")
    }

    pub fn blend_association() -> Option<PathBuf> {
        let key = hkcu().open_subkey_with_flags(format!(r"{CLASSES}\{PROGID}\shell\open\command"), KEY_READ).ok()?;
        let cmd: String = key.get_value("").ok()?;
        exe_of_command(&cmd)
    }

    pub fn set_blend_association(on: bool, app_exe: &Path) -> Result<()> {
        let root = hkcu();
        let open = format!("\"{}\" \"%1\"", app_exe.display());
        let icon = format!("\"{}\",0", app_exe.display());
        if on {
            let (progid, _) = root.create_subkey(format!(r"{CLASSES}\{PROGID}"))?;
            progid.set_value("", &"Blender File")?;
            progid.set_value("FriendlyTypeName", &"Blender File")?;
            progid.create_subkey("DefaultIcon")?.0.set_value("", &icon)?;
            progid.create_subkey(r"shell\open")?.0.set_value("FriendlyAppName", &"Spindle")?;
            progid.create_subkey(r"shell\open\command")?.0.set_value("", &open)?;
            // Offered under "Open with" for .blend without taking over the default app.
            let (owp, _) = root.create_subkey(format!(r"{CLASSES}\.blend\OpenWithProgids"))?;
            owp.set_value(PROGID, &"")?;
            let (app, _) = root.create_subkey(app_key(app_exe))?;
            app.set_value("FriendlyAppName", &"Spindle")?;
            app.create_subkey("SupportedTypes")?.0.set_value(".blend", &"")?;
            app.create_subkey(r"shell\open\command")?.0.set_value("", &open)?;
        } else {
            let _ = root.delete_subkey_all(format!(r"{CLASSES}\{PROGID}"));
            let _ = root.delete_subkey_all(app_key(app_exe));
            if let Ok(owp) = root.open_subkey_with_flags(format!(r"{CLASSES}\.blend\OpenWithProgids"), KEY_READ | KEY_WRITE) {
                let _ = owp.delete_value(PROGID);
            }
        }
        // Tell Explorer the associations changed.
        // SAFETY: documented call with null item pointers for SHCNE_ASSOCCHANGED.
        unsafe {
            use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
            SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());
        }
        Ok(())
    }

    pub fn open_default_apps() -> bool {
        std::process::Command::new("explorer.exe").arg("ms-settings:defaultapps").spawn().is_ok()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Context, TRAY_ARG};
    use anyhow::Result;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn home(rel: &str) -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.home_dir().join(rel))
    }
    fn autostart_file() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.config_dir().join("autostart/spindle.desktop"))
    }
    fn app_file() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.data_dir().join("applications/spindle.desktop"))
    }

    fn exec_of(file: &Path) -> Option<PathBuf> {
        let text = fs::read_to_string(file).ok()?;
        let line = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
        super::exe_of_command(line)
    }

    fn desktop_entry(app_exe: &Path, args: &str, extra: &str) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Spindle\nComment=Blender build, profile and add-on manager\n\
             Exec=\"{}\" {args}\nIcon=spindle\nTerminal=false\nCategories=Graphics;3DGraphics;\n{extra}",
            app_exe.display()
        )
    }

    pub fn autostart() -> Option<PathBuf> {
        exec_of(&autostart_file()?)
    }

    pub fn set_autostart(on: bool, app_exe: &Path) -> Result<()> {
        let file = autostart_file().context("no config directory")?;
        if on {
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, desktop_entry(app_exe, TRAY_ARG, "X-GNOME-Autostart-enabled=true\n"))?;
        } else if file.exists() {
            fs::remove_file(&file)?;
        }
        Ok(())
    }

    pub fn blend_association() -> Option<PathBuf> {
        exec_of(&app_file()?)
    }

    pub fn set_blend_association(on: bool, app_exe: &Path) -> Result<()> {
        let file = app_file().context("no data directory")?;
        if on {
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, desktop_entry(app_exe, "%f", "MimeType=application/x-blender;\n"))?;
            let _ = std::process::Command::new("xdg-mime")
                .args(["default", "spindle.desktop", "application/x-blender"])
                .status();
        } else if file.exists() {
            fs::remove_file(&file)?;
        }
        let _ = home(".local/share/applications").map(|d| {
            std::process::Command::new("update-desktop-database").arg(d).status()
        });
        Ok(())
    }

    pub fn open_default_apps() -> bool {
        false
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Context, TRAY_ARG};
    use anyhow::{Result, bail};
    use std::fs;
    use std::path::{Path, PathBuf};

    const LABEL: &str = "io.github.deepfold9118.spindle";

    fn agent_file() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.home_dir().join(format!("Library/LaunchAgents/{LABEL}.plist")))
    }

    pub fn autostart() -> Option<PathBuf> {
        let text = fs::read_to_string(agent_file()?).ok()?;
        let start = text.find("<string>")? + 8;
        let after_label = &text[start..];
        // The second <string> is the first program argument (the executable).
        let rest = &after_label[after_label.find("<string>")? + 8..];
        Some(PathBuf::from(&rest[..rest.find("</string>")?]))
    }

    pub fn set_autostart(on: bool, app_exe: &Path) -> Result<()> {
        let file = agent_file().context("no home directory")?;
        if on {
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(
                &file,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                     <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                     <plist version=\"1.0\"><dict>\n\
                     <key>Label</key><string>{LABEL}</string>\n\
                     <key>ProgramArguments</key><array><string>{}</string><string>{TRAY_ARG}</string></array>\n\
                     <key>RunAtLoad</key><true/>\n\
                     </dict></plist>\n",
                    app_exe.display()
                ),
            )?;
        } else if file.exists() {
            fs::remove_file(&file)?;
        }
        Ok(())
    }

    pub fn blend_association() -> Option<PathBuf> {
        None
    }

    pub fn set_blend_association(on: bool, _app_exe: &Path) -> Result<()> {
        if on {
            bail!("on macOS, file types are registered by app bundles; Spindle ships as plain binaries, so use `spindle open <file>` instead");
        }
        Ok(())
    }

    pub fn open_default_apps() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_from_command_lines() {
        assert_eq!(exe_of_command(r#""C:\a b\spindle_app.exe" --tray"#), Some(PathBuf::from(r"C:\a b\spindle_app.exe")));
        assert_eq!(exe_of_command(r#""C:\x\spindle_app.exe" "%1""#), Some(PathBuf::from(r"C:\x\spindle_app.exe")));
        assert_eq!(exe_of_command("/usr/bin/spindle_app %f"), Some(PathBuf::from("/usr/bin/spindle_app")));
        assert_eq!(exe_of_command(""), None);
    }
}
