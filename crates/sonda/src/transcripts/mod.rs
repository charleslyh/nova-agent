//! Session event transcripts: persisted per-session event log with subscribe, load, and write.
//!
//! Default on-disk layout uses the [`jsonl`] backend under `{sessions_dir}/{session_id}/`.

use moray_session::SessionEvent;

mod jsonl;
mod replay;
mod store;

/// One persisted transcript row: monotonic `seq` plus the domain [`SessionEvent`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SondaSessionEventRecord {
    pub seq: u64,
    pub event: SessionEvent,
}

pub use replay::replay_records;
pub use store::{SondaSessionTranscriptsError, SondaSessionTranscripts};
