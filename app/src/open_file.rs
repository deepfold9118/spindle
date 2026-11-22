//! `spindle_app file.blend`: what runs when a .blend is opened with Spindle from the file
//! manager. Picks the build for the file, checks the profile, asks only when the build can't
//! use the profile or the file is newer than every build, launches Blender and exits without
//! opening Spindle's window.

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use spindle_core::profile::Profile;
use spindle_core::{Spindle, blendfile, compat, launch};
use std::path::{Path, PathBuf};

/// The .blend file among the command-line arguments, if any.
pub fn blend_arg(args: &[String]) -> Option<PathBuf> {
    args.iter().skip(1).filter(|a| !a.starts_with("--")).map(PathBuf::from).find(|p| {
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case("blend")) && p.is_file()
    })
}

fn error(text: &str) {
    MessageDialog::new().set_level(MessageLevel::Error).set_title("Spindle").set_description(text).set_buttons(MessageButtons::Ok).show();
}

/// Open `file` in the build that suits it. Errors are shown in a message box.
pub fn open(file: &Path) {
    if let Err(e) = try_open(file) {
        error(&format!("Couldn't open {}:\n\n{e:#}", file.display()));
    }
}

fn try_open(file: &Path) -> anyhow::Result<()> {
    let sp = Spindle::load()?;
    let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let profile = Profile::open(&sp, &sp.profile_name(None))?;
    let usable = |b: &spindle_core::builds::Build| {
        compat::check(b, &profile.name, &profile.root).worst() != Some(compat::Severity::Blocker)
    };
    let choice = blendfile::choose_build(&sp, &file, usable)?;
    let report = compat::check(&choice.build, &profile.name, &profile.root);

    let mut notes: Vec<String> = choice.warning.iter().cloned().collect();
    notes.extend(
        report
            .issues
            .iter()
            // Warnings (an add-on that won't load, say) are shown in Spindle, not on every open.
            .filter(|i| i.severity == compat::Severity::Blocker)
            .map(|i| format!("{}: {}", i.title, i.detail)),
    );
    if !notes.is_empty() {
        let version = choice.build.version.clone().unwrap_or_else(|| choice.build.name.clone());
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let text = format!(
            "Opening {name} in Blender {version} with profile '{}':\n\n- {}",
            profile.name,
            notes.join("\n\n- ")
        );
        let open = "Open Anyway".to_string();
        let answer = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title("Spindle")
            .set_description(text)
            .set_buttons(MessageButtons::OkCancelCustom(open.clone(), "Cancel".into()))
            .show();
        if answer != MessageDialogResult::Custom(open) && answer != MessageDialogResult::Ok {
            return Ok(());
        }
    }
    launch::launch(&sp, &choice.build, &profile, &[file.to_string_lossy().into_owned()])?;
    Ok(())
}
