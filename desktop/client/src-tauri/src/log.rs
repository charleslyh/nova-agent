//! Tracing subscriber for the desktop application (stderr).

use std::io;

/// Install a global tracing subscriber writing to stderr.
///
/// Respects `RUST_LOG` when set; otherwise defaults to `info`.
/// Safe to call more than once: subsequent calls are ignored.
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let _ = tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();
}
