//! Spindle core: build library, shared profiles, launching, running-instance
//! discovery, the local add-on library, the Superhive and GitHub add-on sources, Blender build
//! downloads and profile compatibility checks.

pub mod addons;
pub mod backup;
pub mod blendfile;
pub mod builds;
pub mod compat;
pub mod config;
pub mod desktop;
pub mod downloads;
pub mod github;
pub mod instances;
pub mod launch;
pub mod profile;
pub mod selfupdate;
pub mod superhive;
pub mod version;

pub use config::Spindle;
