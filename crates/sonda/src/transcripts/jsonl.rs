//! JSONL transcript file backend (default on-disk format).

use std::io::Write;
use std::path::{Path, PathBuf};

use super::SondaSessionEventRecord;

pub(crate) const TRANSCRIPT_FILE: &str = "transcript.jsonl";

const SCHEMA_CURRENT: u32 = 1;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FileHeader {
    moray_transcript_schema: u32,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum JsonlError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported transcript schema version {0} (expected {SCHEMA_CURRENT})")]
    UnsupportedSchema(u32),
    #[error("invalid transcript: {0}")]
    InvalidFormat(String),
}

pub(crate) fn session_transcript_path(sessions_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir
        .join(session_id)
        .join(TRANSCRIPT_FILE)
}

pub(crate) fn create_transcript_file(path: &Path) -> Result<(), JsonlError> {
    if path.exists() {
        return Err(JsonlError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "transcript file already exists",
        )));
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::File::create(path)?;
    write_header_line(&mut f, SCHEMA_CURRENT)?;
    Ok(())
}

pub(crate) fn read_transcript(path: &Path) -> Result<Vec<SondaSessionEventRecord>, JsonlError> {
    if !path.exists() {
        return Err(JsonlError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "transcript file not found",
        )));
    }
    let raw = std::fs::read_to_string(path)?;
    parse_records_str(&raw)
}

/// Replace the on-disk transcript with a fresh header and the given records (seqs as provided).
pub(crate) fn write_transcript(
    path: &Path,
    records: &[SondaSessionEventRecord],
) -> Result<(), JsonlError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::File::create(path)?;
    write_header_line(&mut f, SCHEMA_CURRENT)?;
    for record in records {
        writeln!(f, "{}", serde_json::to_string(record)?)?;
    }
    Ok(())
}

pub(crate) fn append_record(
    path: &Path,
    record: &SondaSessionEventRecord,
) -> Result<(), JsonlError> {
    let mut f = std::fs::OpenOptions::new().append(true).open(path)?;
    if f.metadata()?.len() == 0 {
        write_header_line(&mut f, SCHEMA_CURRENT)?;
    }
    writeln!(f, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

pub(crate) fn next_seq_after(records: &[SondaSessionEventRecord]) -> u64 {
    records.last().map(|r| r.seq + 1).unwrap_or(1)
}

fn write_header_line(w: &mut dyn Write, schema: u32) -> Result<(), JsonlError> {
    let line = format!(
        "{}\n",
        serde_json::to_string(&FileHeader {
            moray_transcript_schema: schema,
        })?
    );
    w.write_all(line.as_bytes())?;
    Ok(())
}

pub(crate) fn parse_records_str(raw: &str) -> Result<Vec<SondaSessionEventRecord>, JsonlError> {
    let mut out = Vec::new();
    let mut saw_header = false;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(header) = serde_json::from_str::<FileHeader>(line) {
            if saw_header {
                return Err(JsonlError::InvalidFormat(
                    "duplicate transcript header".into(),
                ));
            }
            if header.moray_transcript_schema != SCHEMA_CURRENT {
                return Err(JsonlError::UnsupportedSchema(
                    header.moray_transcript_schema,
                ));
            }
            saw_header = true;
            continue;
        }
        let record = serde_json::from_str::<SondaSessionEventRecord>(line)?;
        out.push(record);
    }
    if !out.is_empty() && !saw_header {
        return Err(JsonlError::InvalidFormat(
            "transcript records require a header line".into(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moray_session::{SessionEvent, SessionEventKind, TurnInput};

    fn wrap_user(content: &str) -> SessionEvent {
        SessionEvent {
            session_id: "test-session".into(),
            ts: 1,
            kind: SessionEventKind::TurnAccepted {
                input: TurnInput {
                    text: content.into(),
                    resources: Vec::new(),
                },
            },
        }
    }

    #[test]
    fn parse_skips_file_header() {
        let h = FileHeader {
            moray_transcript_schema: 1,
        };
        let event = wrap_user("hi");
        let record = SondaSessionEventRecord { seq: 1, event };
        let mut s = String::new();
        s.push_str(&serde_json::to_string(&h).expect("serialize header"));
        s.push('\n');
        s.push_str(&serde_json::to_string(&record).expect("serialize record"));
        s.push('\n');
        let out = parse_records_str(&s).expect("parse transcript");
        assert_eq!(out, vec![record]);
    }

    #[test]
    fn parse_rejects_bare_session_event_lines() {
        let line = serde_json::to_string(&wrap_user("hi")).expect("serialize event");
        let err = parse_records_str(&line).unwrap_err();
        assert!(matches!(
            err,
            JsonlError::InvalidFormat(_) | JsonlError::Json(_)
        ));
    }

    #[test]
    fn parse_rejects_unsupported_schema_version() {
        let h = FileHeader {
            moray_transcript_schema: 99,
        };
        let raw = serde_json::to_string(&h).expect("serialize header");
        let err = parse_records_str(&raw).unwrap_err();
        assert!(matches!(err, JsonlError::UnsupportedSchema(99)));
    }
}
