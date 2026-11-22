use crate::builds::Build;
use crate::profile::Profile;
use crate::Spindle;
use anyhow::{Context, Result, bail};
use std::fs;
use std::process::{Command, Output, Stdio};

fn command(sp: &Spindle, build: &Build, profile: &Profile, args: &[String]) -> Result<Command> {
    fs::create_dir_all(sp.instances_dir())?;
    let mut cmd = Command::new(&build.executable);
    cmd.args(args).envs(profile.env(sp, build));
    if let Some(dir) = build.executable.parent() {
        cmd.current_dir(dir);
    }
    Ok(cmd)
}

/// Windows creates children with handle inheritance on, so a detached Blender would
/// otherwise inherit Spindle's own stdio pipes and keep them open for its whole lifetime.
/// Anything reading Spindle's output (a shell capturing `spindle launch`, or an MCP
/// client talking to `spindle-mcp` over stdio) would then block until Blender exits.
#[cfg(windows)]
fn stop_stdio_inheritance() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: plain Win32 calls on this process's std handles; failures (e.g. no
        // console handle) are harmless and ignored.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// Start an interactive Blender, detached from Spindle. Returns the process id.
/// Preferences a newer Blender saved are backed up first (see `backup::before_launch`).
pub fn launch(sp: &Spindle, build: &Build, profile: &Profile, args: &[String]) -> Result<u32> {
    // A failed backup doesn't stop the launch; the compatibility check has already warned.
    let _ = crate::backup::before_launch(sp, build, profile);
    #[cfg(windows)]
    stop_stdio_inheritance();
    let mut cmd = command(sp, build, profile, args)?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    let child = cmd.spawn().with_context(|| format!("starting {}", build.executable.display()))?;
    Ok(child.id())
}

/// Run Blender headless (`-b` is added) with the profile and wait for it.
pub fn run_background(sp: &Spindle, build: &Build, profile: &Profile, args: &[String]) -> Result<Output> {
    let mut full = vec!["-b".to_owned()];
    full.extend_from_slice(args);
    let out = command(sp, build, profile, &full)?
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {}", build.executable.display()))?;
    if !out.status.success() {
        bail!(
            "Blender exited with {}:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim_end()
        );
    }
    Ok(out)
}
