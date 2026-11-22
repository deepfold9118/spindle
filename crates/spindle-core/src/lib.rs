//! Spindle core: build library, shared profiles, launching, running-instance
//! discovery, the local add-on library, and the Superhive download source.

pub mod addons;
pub mod builds;
pub mod config;
pub mod instances;
pub mod launch;
pub mod profile;
pub mod superhive;
pub mod version;

pub use config::Spindle;
