//! Moray desktop client library (Tauri app wiring).

pub mod log;
pub mod sonda;

#[cfg(feature = "tauri-app")]
pub mod bundle;
