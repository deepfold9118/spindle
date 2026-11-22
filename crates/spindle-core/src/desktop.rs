//! Desktop integration: starting Spindle when the user signs in, opening `.blend` files
//! with Spindle (which picks a build for each file, see `blendfile`), and putting the
//! programs on PATH.
//!
//! - Windows: `HKCU\...\CurrentVersion\Run`, an "Open with" registration under
//!   `HKCU\Software\Classes`, and the user `Path` (`HKCU\Environment`). Windows doesn't let
//!   programs make themselves the default app; `open_default_apps()` opens the Settings page
//!   where the user picks Spindle.
//! - Linux: `~/.config/autostart/spindle.desktop`, `~/.local/share/applications/
//!   spindle.desktop` with the `application/x-blender` MIME type (also made the default),
//!   and links in `~/.local/bin`.
//! - macOS: a LaunchAgent for signing in and links in `~/.local/bin`. File associations
//!   need an app bundle, which Spindle doesn't ship yet, so `set_blend_association` says so.

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

/// Names of the programs in an install folder (with the platform's .exe suffix).
pub fn program_names() -> [String; 3] {
    ["spindle", "spindle_app", "spindle-mcp"].map(|b| format!("{b}{}", std::env::consts::EXE_SUFFIX))
}

/// The Spindle folder on PATH, if any: on Windows the user `Path` entry holding `spindle.exe`,
/// elsewhere the folder `~/.local/bin/spindle` links to.
pub fn path_dir() -> Option<PathBuf> {
    imp::path_dir()
}

/// Whether the programs in `dir` are on PATH (a later entry than `path_dir` counts too).
pub fn is_on_path(dir: &Path) -> bool {
    imp::is_on_path(dir)
}

/// Put the programs in `dir` on PATH (or take them off). Windows: an entry in the user
/// `Path` (new terminals see it). Linux/macOS: links in `~/.local/bin`.
pub fn set_on_path(on: bool, dir: &Path) -> Result<()> {
    imp::set_on_path(on, dir)
}

/// Where Linux/macOS links go, and whether it's on PATH already (it isn't on stock macOS).
pub fn user_bin() -> Option<(PathBuf, bool)> {
    let bin = directories::BaseDirs::new()?.home_dir().join(".local").join("bin");
    let on = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == bin));
    Some((bin, on))
}

/// Ask Explorer to refresh its icon cache. It caches program icons by path, so after an
/// update replaces the programs the taskbar could keep the previous release's icon.
pub fn refresh_icons() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Some(root) = std::env::var_os("SystemRoot") {
            let tool = Path::new(&root).join(r"System32\ie4uinit.exe");
            let _ = std::process::Command::new(tool).arg("-show").creation_flags(CREATE_NO_WINDOW).spawn();
        }
    }
}

/// Re-point registrations at `app_exe` when the copy they point at is gone (Spindle moved).
/// Registrations of another copy that still exists are left alone, so starting a second
/// copy (a development build, say) doesn't take them over.
pub fn refresh(app_exe: &Path) {
    let gone = |p: &Path| p != app_exe && !p.exists();
    if autostart().is_some_and(|p| gone(&p)) {
        let _ = set_autostart(true, app_exe);
    }
    if blend_association().is_some_and(|p| gone(&p)) {
        let _ = set_blend_association(true, app_exe);
    }
    if let (Some(old), Some(dir)) = (path_dir(), app_exe.parent()) {
        if old != dir && !old.join(&program_names()[0]).exists() {
            let _ = set_on_path(false, &old);
            let _ = set_on_path(true, dir);
        }
    }
}

/// Unix: the folder a `~/.local/bin` link points into, when it's one of Spindle's programs.
#[cfg(not(windows))]
fn linked_dir() -> Option<PathBuf> {
    let (bin, _) = user_bin()?;
    let target = std::fs::read_link(bin.join("spindle")).ok()?;
    target.parent().map(Path::to_path_buf)
}

#[cfg(not(windows))]
fn set_links(on: bool, dir: &Path) -> Result<()> {
    let (bin, _) = user_bin().context("no home directory")?;
    for name in program_names() {
        let link = bin.join(&name);
        let ours = std::fs::read_link(&link).is_ok_and(|t| t.parent() == Some(dir));
        if on {
            std::fs::create_dir_all(&bin)?;
            if std::fs::symlink_metadata(&link).is_ok() {
                if std::fs::read_link(&link).is_err() {
                    anyhow::bail!("{} exists and isn't a link; not replacing it", link.display());
                }
                std::fs::remove_file(&link)?;
            }
            std::os::unix::fs::symlink(dir.join(&name), &link)
                .with_context(|| format!("linking {}", link.display()))?;
        } else if ours {
            std::fs::remove_file(&link)?;
        }
    }
    Ok(())
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

    /// `%LOCALAPPDATA%\Programs\Spindle` -> `C:\Users\me\AppData\Local\Programs\Spindle`
    fn expand(entry: &str) -> String {
        let mut out = String::new();
        let mut rest = entry;
        while let Some(start) = rest.find('%') {
            let Some(len) = rest[start + 1..].find('%') else { break };
            let name = &rest[start + 1..start + 1 + len];
            out.push_str(&rest[..start]);
            match std::env::var(name) {
                Ok(v) => out.push_str(&v),
                Err(_) => out.push_str(&rest[start..start + len + 2]),
            }
            rest = &rest[start + len + 2..];
        }
        out.push_str(rest);
        out
    }

    fn same_dir(entry: &str, dir: &Path) -> bool {
        let e = expand(entry);
        e.trim_end_matches('\\').eq_ignore_ascii_case(dir.to_string_lossy().trim_end_matches('\\'))
    }

    /// The user Path, unexpanded, and whether it's REG_EXPAND_SZ.
    fn user_path() -> (Vec<String>, bool) {
        let Ok(env) = hkcu().open_subkey_with_flags("Environment", KEY_READ) else { return (Vec::new(), true) };
        match env.get_raw_value("Path") {
            Ok(v) => {
                let wide: Vec<u16> = v.bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let text = String::from_utf16_lossy(&wide);
                let text = text.trim_end_matches('\0');
                let entries = text.split(';').filter(|e| !e.is_empty()).map(str::to_owned).collect();
                (entries, v.vtype == winreg::enums::REG_EXPAND_SZ)
            }
            Err(_) => (Vec::new(), true),
        }
    }

    pub fn path_dir() -> Option<PathBuf> {
        let (entries, _) = user_path();
        entries.iter().map(|e| PathBuf::from(expand(e))).find(|d| d.join("spindle.exe").is_file())
    }

    pub fn is_on_path(dir: &Path) -> bool {
        user_path().0.iter().any(|e| same_dir(e, dir))
    }

    pub fn set_on_path(on: bool, dir: &Path) -> Result<()> {
        let (mut entries, expand_sz) = user_path();
        let had = entries.iter().any(|e| same_dir(e, dir));
        if on == had {
            return Ok(());
        }
        if on {
            // Store under %LOCALAPPDATA% when it's there, like Windows' own entries.
            let shown = match std::env::var("LOCALAPPDATA") {
                Ok(l) if dir.starts_with(&l) => {
                    format!("%LOCALAPPDATA%{}", &dir.to_string_lossy()[l.len()..])
                }
                _ => dir.display().to_string(),
            };
            entries.push(shown);
        } else {
            entries.retain(|e| !same_dir(e, dir));
        }
        let value = entries.join(";");
        let mut bytes: Vec<u8> = value.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
        if bytes.is_empty() {
            bytes = vec![0, 0];
        }
        let vtype = if expand_sz || value.contains('%') { winreg::enums::REG_EXPAND_SZ } else { winreg::enums::REG_SZ };
        let (env, _) = hkcu().create_subkey("Environment")?;
        env.set_raw_value("Path", &winreg::RegValue { bytes: bytes.into(), vtype }).context("writing the user Path")?;
        // Tell Explorer (and so new terminals) the environment changed.
        let what: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: documented broadcast with a NUL-terminated string and a timeout.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE};
            let mut result = 0usize;
            SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, what.as_ptr() as isize, SMTO_ABORTIFHUNG, 5000, &mut result);
        }
        Ok(())
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

    pub fn path_dir() -> Option<PathBuf> {
        super::linked_dir()
    }

    pub fn is_on_path(dir: &Path) -> bool {
        super::linked_dir().as_deref() == Some(dir)
    }

    pub fn set_on_path(on: bool, dir: &Path) -> Result<()> {
        super::set_links(on, dir)
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

    pub fn path_dir() -> Option<PathBuf> {
        super::linked_dir()
    }

    pub fn is_on_path(dir: &Path) -> bool {
        super::linked_dir().as_deref() == Some(dir)
    }

    pub fn set_on_path(on: bool, dir: &Path) -> Result<()> {
        super::set_links(on, dir)
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
