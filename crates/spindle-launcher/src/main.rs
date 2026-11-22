use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use spindle_core::addons::AddonLibrary;
use spindle_core::profile::Profile;
use spindle_core::{Spindle, builds, instances, launch, superhive};
use std::io::Read;
use std::path::PathBuf;

/// Spindle: a build, profile and add-on manager for Blender.
#[derive(Parser)]
#[command(name = "spindle", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show where Spindle keeps its config and library.
    Info,
    /// Set the library folder (builds, add-ons, profiles).
    SetLibrary { dir: PathBuf },
    #[command(subcommand)]
    Builds(BuildsCmd),
    /// List profiles.
    Profiles,
    /// Launch a build with a shared profile.
    Launch {
        /// Build name, or a version like "5.3".
        build: String,
        #[arg(long, short)]
        profile: Option<String>,
        /// Extra Blender arguments (after `--`).
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// List Blenders launched by Spindle that are running right now.
    Instances {
        #[arg(long)]
        json: bool,
    },
    #[command(subcommand)]
    Addons(AddonsCmd),
    #[command(subcommand)]
    Superhive(SuperhiveCmd),
}

#[derive(Subcommand)]
enum BuildsCmd {
    /// List builds in the library and registered external builds.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Register a Blender executable that lives outside the library.
    Add { name: String, executable: PathBuf },
}

#[derive(Subcommand)]
enum AddonsCmd {
    /// List archives in the add-on library.
    List,
    /// Install the newest stored version of an add-on into a profile.
    Install {
        id: String,
        /// Build whose extension tooling performs the install.
        #[arg(long, short)]
        build: String,
        #[arg(long, short)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum SuperhiveCmd {
    /// Store your Superhive API token in the OS credential store.
    Login {
        /// Read the token from stdin instead of prompting.
        #[arg(long)]
        stdin: bool,
    },
    /// Remove the stored token.
    Logout,
    /// Show purchased extensions available for this platform.
    List,
    /// Download purchased extensions that aren't in the add-on library yet.
    Sync,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut sp = Spindle::load()?;
    match cli.command {
        Cmd::Info => {
            println!("config:    {}", sp.config_path.display());
            println!("library:   {}", sp.library.display());
            println!("builds:    {}", sp.builds_dir().display());
            println!("addons:    {}", sp.addons_dir().display());
            println!("profiles:  {}", sp.profiles_dir().display());
            println!("instances: {}", sp.instances_dir().display());
            println!("superhive: {} (token {})", sp.config.superhive.repo_url,
                if superhive::has_token() { "stored" } else { "not set" });
        }
        Cmd::SetLibrary { dir } => {
            sp.config.library_dir = Some(dir.clone());
            sp.save()?;
            println!("library set to {}", dir.display());
        }
        Cmd::Builds(BuildsCmd::List { json: true }) => {
            println!("{}", serde_json_pretty(&builds::list(&sp)?)?);
        }
        Cmd::Builds(BuildsCmd::List { json: false }) => {
            for b in builds::list(&sp)? {
                println!("{:<10} {}{}", b.version.as_deref().unwrap_or("?"), b.name,
                    if b.external { format!("  ({})", b.executable.display()) } else { String::new() });
            }
        }
        Cmd::Builds(BuildsCmd::Add { name, executable }) => {
            builds::register_external(&mut sp, &name, &executable)?;
            println!("registered {name}");
        }
        Cmd::Profiles => {
            for p in Profile::list(&sp)? {
                println!("{p}");
            }
        }
        Cmd::Launch { build, profile, args } => {
            let build = builds::find(&sp, &build)?;
            let profile = Profile::open(&sp, &sp.profile_name(profile.as_deref()))?;
            let pid = launch::launch(&sp, &build, &profile, &args)?;
            println!("launched {} with profile '{}' (pid {pid})", build.name, profile.name);
        }
        Cmd::Instances { json } => {
            let list = instances::list(&sp)?;
            if json {
                println!("{}", serde_json_pretty(&list)?);
            } else if list.is_empty() {
                println!("no running Blender launched by Spindle");
            } else {
                for i in list {
                    let mcp: Vec<String> = i.mcp.iter()
                        .map(|m| format!("{}@{}:{}", m.addon, m.host.as_deref().unwrap_or("?"),
                            m.port.map(|p| p.to_string()).unwrap_or("?".into())))
                        .collect();
                    println!("pid {:<7} Blender {:<8} profile {:<10} file {}  mcp [{}]",
                        i.pid, i.blender_version, i.profile.as_deref().unwrap_or("-"),
                        if i.file.is_empty() { "(unsaved)" } else { &i.file }, mcp.join(", "));
                }
            }
        }
        Cmd::Addons(AddonsCmd::List) => {
            let lib = AddonLibrary::new(&sp);
            for r in lib.list()? {
                println!("{:<28} {:<12} {:<8} {:<10} {}", r.id, r.version, r.kind, r.source, r.archive);
            }
        }
        Cmd::Addons(AddonsCmd::Install { id, build, profile }) => {
            let lib = AddonLibrary::new(&sp);
            let rec = lib.latest(&id)?.with_context(|| format!("'{id}' is not in the add-on library"))?;
            let build = builds::find(&sp, &build)?;
            let profile = Profile::open(&sp, &sp.profile_name(profile.as_deref()))?;
            lib.install(&sp, &build, &profile, &rec)?;
            println!("installed {} {} into profile '{}'", rec.id, rec.version, profile.name);
        }
        Cmd::Superhive(SuperhiveCmd::Login { stdin }) => {
            let token = if stdin {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                rpassword::prompt_password("Superhive API token: ")?
            };
            superhive::set_token(&token)?;
            println!("token stored in the OS credential store");
        }
        Cmd::Superhive(SuperhiveCmd::Logout) => {
            superhive::clear_token()?;
            println!("token removed");
        }
        Cmd::Superhive(SuperhiveCmd::List) => {
            let lib = AddonLibrary::new(&sp);
            for e in superhive::fetch_index(&sp)? {
                let have = lib.contains(superhive::SOURCE, &e.id, &e.version);
                println!("{:<28} {:<12} {:<8} {}", e.id, e.version, e.kind, if have { "in library" } else { "new" });
            }
        }
        Cmd::Superhive(SuperhiveCmd::Sync) => {
            let report = superhive::sync(&sp, &AddonLibrary::new(&sp))?;
            for r in &report.downloaded {
                println!("downloaded  {} {}", r.id, r.version);
            }
            for s in &report.up_to_date {
                println!("up to date  {s}");
            }
            for (what, err) in &report.failed {
                println!("FAILED      {what}: {err}");
            }
            if !report.failed.is_empty() {
                bail!("{} download(s) failed", report.failed.len());
            }
        }
    }
    Ok(())
}

fn serde_json_pretty<T: serde::Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(v)?)
}
