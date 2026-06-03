//! Session event transcripts: per-session persisted event log with subscribe, load, and write.
//!
//! Callers use [`SondaSessionTranscripts::new`] with a resolved `sessions` directory.
//! On-disk layout is `{sessions_dir}/{session_id}/transcript.jsonl` (see [`jsonl`] backend).

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use moray_channels::{SessionLiveEvents, SessionLiveEventsError};
use moray_core::MorayError;
use moray_session::{SessionEvent, SessionEventKind, SessionEventSink};
use thiserror::Error;
use tokio::sync::mpsc;

use super::jsonl::{self, JsonlError};
use super::SondaSessionEventRecord;

/// Transcript store errors (missing session, duplicate create, backend I/O).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SondaSessionTranscriptsError {
    #[error("transcript not found")]
    NotFound,
    #[error("transcript already exists")]
    AlreadyExists,
    #[error("{0}")]
    Message(String),
}

impl From<SondaSessionTranscriptsError> for MorayError {
    fn from(value: SondaSessionTranscriptsError) -> Self {
        Self::Message(value.to_string())
    }
}

impl From<SondaSessionTranscriptsError> for SessionLiveEventsError {
    fn from(err: SondaSessionTranscriptsError) -> Self {
        match err {
            SondaSessionTranscriptsError::NotFound => SessionLiveEventsError::NotFound,
            SondaSessionTranscriptsError::AlreadyExists => {
                SessionLiveEventsError::Message("transcript already exists".into())
            }
            SondaSessionTranscriptsError::Message(msg) => SessionLiveEventsError::Message(msg),
        }
    }
}

fn backend_err(e: JsonlError) -> SondaSessionTranscriptsError {
    SondaSessionTranscriptsError::Message(e.to_string())
}

/// Activated per-session transcript runtime: append, replay subscribe, live fan-out.
#[derive(Debug)]
struct TranscriptRuntime {
    path: PathBuf,
    state: Arc<Mutex<RuntimeState>>,
}

#[derive(Debug)]
struct RuntimeState {
    next_seq: u64,
    subscribers: Vec<Subscriber>,
}

#[derive(Debug)]
struct Subscriber {
    start_seq: u64,
    tx: mpsc::UnboundedSender<SondaSessionEventRecord>,
}

impl TranscriptRuntime {
    fn new(path: PathBuf) -> Self {
        let next_seq = jsonl::read_transcript(&path)
            .map(|records| jsonl::next_seq_after(&records))
            .unwrap_or(1);
        Self {
            path,
            state: Arc::new(Mutex::new(RuntimeState {
                next_seq,
                subscribers: Vec::new(),
            })),
        }
    }

    fn load(&self) -> Result<(Vec<SondaSessionEventRecord>, u64), JsonlError> {
        let records = jsonl::read_transcript(&self.path)?;
        let next_seq = jsonl::next_seq_after(&records);
        Ok((records, next_seq))
    }

    fn subscribe(
        &self,
        from_seq: u64,
    ) -> Result<mpsc::UnboundedReceiver<SondaSessionEventRecord>, JsonlError> {
        let (tx, rx) = mpsc::unbounded_channel();

        let live_start = {
            let mut state = self.state.lock().expect("transcript runtime state poisoned");
            let live_start = state.next_seq;
            state.subscribers.push(Subscriber {
                start_seq: live_start,
                tx: tx.clone(),
            });
            live_start
        };

        let (records, _) = self.load()?;
        for record in records {
            if record.seq >= from_seq && record.seq < live_start && tx.send(record).is_err() {
                return Ok(rx);
            }
        }

        Ok(rx)
    }

    /// Live tail only — no persisted replay. For IM outbound subscribers that must not
    /// re-emit historical agent turns on channel (re)start.
    fn subscribe_live(&self) -> mpsc::UnboundedReceiver<SondaSessionEventRecord> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut state = self.state.lock().expect("transcript runtime state poisoned");
        let live_start = state.next_seq;
        state.subscribers.push(Subscriber {
            start_seq: live_start,
            tx,
        });
        rx
    }

    fn append_record(&self, event: &SessionEvent) -> Result<SondaSessionEventRecord, JsonlError> {
        let seq = {
            let mut state = self.state.lock().expect("transcript runtime state poisoned");
            let seq = state.next_seq;
            state.next_seq += 1;
            seq
        };
        let record = SondaSessionEventRecord {
            seq,
            event: event.clone(),
        };
        jsonl::append_record(&self.path, &record)?;
        Ok(record)
    }

    fn publish_delta(&self, delta: SondaSessionEventRecord) {
        let is_reset = matches!(delta.event.kind, SessionEventKind::Reset);
        let mut state = self.state.lock().expect("transcript runtime state poisoned");
        state.subscribers.retain_mut(|sub| {
            if is_reset {
                // Reset rewrites the transcript starting at seq 1; rewind the
                // subscriber cursor so post-reset events are not dropped.
                sub.start_seq = delta.seq;
                return sub.tx.send(delta.clone()).is_ok();
            }
            if delta.seq < sub.start_seq {
                return true;
            }
            sub.tx.send(delta.clone()).is_ok()
        });
    }
}

impl TranscriptRuntime {
    fn append(&self, event: &SessionEvent) -> Result<(), MorayError> {
        if matches!(event.kind, SessionEventKind::Reset) {
            let delta = SondaSessionEventRecord {
                seq: 1,
                event: event.clone(),
            };
            jsonl::write_transcript(&self.path, std::slice::from_ref(&delta))
                .map_err(|e| MorayError::Message(e.to_string()))?;
            {
                let mut state = self.state.lock().expect("transcript runtime state poisoned");
                state.next_seq = 2;
            }
            self.publish_delta(delta);
            return Ok(());
        }

        let delta = self
            .append_record(event)
            .map_err(|e| MorayError::Message(e.to_string()))?;
        self.publish_delta(delta);
        Ok(())
    }

}

/// Multi-session transcript registry; activates a [`SondaTranscriptRuntime`] per session on demand.
pub struct SondaSessionTranscripts {
    sessions_dir: PathBuf,
    runtimes: Mutex<HashMap<String, Arc<TranscriptRuntime>>>,
    sink_hook: Mutex<Option<Arc<dyn SessionEventSink>>>,
}

impl SondaSessionTranscripts {
    pub fn new(sessions_dir: impl Into<PathBuf>) -> Self {
        Self {
            sessions_dir: sessions_dir.into(),
            runtimes: Mutex::new(HashMap::new()),
            sink_hook: Mutex::new(None),
        }
    }

    pub fn set_hook(&self, hook: Arc<dyn SessionEventSink>) {
        *self
            .sink_hook
            .lock()
            .expect("SondaSessionTranscripts sink hook lock poisoned") = Some(hook);
    }

    pub fn exists(&self, session_id: &str) -> bool {
        self.transcript_path(session_id).exists()
    }

    pub fn create(&self, session_id: &str) -> Result<(), SondaSessionTranscriptsError> {
        let path = self.transcript_path(session_id);
        if path.exists() {
            return Err(SondaSessionTranscriptsError::AlreadyExists);
        }
        jsonl::create_transcript_file(&path).map_err(backend_err)?;
        Ok(())
    }

    pub fn remove(&self, session_id: &str) -> Result<(), SondaSessionTranscriptsError> {
        let path = self.transcript_path(session_id);
        if !path.exists() {
            return Err(SondaSessionTranscriptsError::NotFound);
        }

        self.runtimes
            .lock()
            .expect("SondaSessionTranscripts runtimes map poisoned")
            .remove(session_id);

        let session_dir = path.parent().ok_or_else(|| {
            SondaSessionTranscriptsError::Message("invalid transcript path".into())
        })?;
        std::fs::remove_dir_all(session_dir).map_err(|e| {
            SondaSessionTranscriptsError::Message(format!("remove session transcript dir: {e}"))
        })?;

        Ok(())
    }

    pub fn load(
        &self,
        session_id: &str,
    ) -> Result<(Vec<SondaSessionEventRecord>, u64), SondaSessionTranscriptsError> {
        self.runtime(session_id)?.load().map_err(backend_err)
    }

    pub fn subscribe(
        &self,
        session_id: &str,
        from_seq: u64,
    ) -> Result<mpsc::UnboundedReceiver<SondaSessionEventRecord>, SondaSessionTranscriptsError> {
        self.runtime(session_id)?
            .subscribe(from_seq)
            .map_err(backend_err)
    }

    /// Subscribe to live session events only (no transcript replay).
    pub fn subscribe_live(
        &self,
        session_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<SondaSessionEventRecord>, SondaSessionTranscriptsError> {
        Ok(self.runtime(session_id)?.subscribe_live())
    }

    fn runtime(
        &self,
        session_id: &str,
    ) -> Result<Arc<TranscriptRuntime>, SondaSessionTranscriptsError> {
        let path = self.transcript_path(session_id);
        if !path.exists() {
            return Err(SondaSessionTranscriptsError::NotFound);
        }

        let mut runtimes = self
            .runtimes
            .lock()
            .expect("SondaSessionTranscripts runtimes map poisoned");

        match runtimes.entry(session_id.to_string()) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let runtime = Arc::new(TranscriptRuntime::new(path));
                Ok(entry.insert(runtime).clone())
            }
        }
    }

    fn transcript_path(&self, session_id: &str) -> PathBuf {
        jsonl::session_transcript_path(&self.sessions_dir, session_id)
    }
}

impl SessionEventSink for SondaSessionTranscripts {
    fn append(&self, event: &SessionEvent) -> Result<(), MorayError> {
        self.runtime(event.session_id.as_str())?.append(event)?;
        if let Some(hook) = self
            .sink_hook
            .lock()
            .expect("SondaSessionTranscripts sink hook lock poisoned")
            .as_ref()
            .cloned()
        {
            hook.append(event)?;
        }
        Ok(())
    }
}

impl SessionLiveEvents for SondaSessionTranscripts {
    fn subscribe_live(
        &self,
        session_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<SessionEvent>, SessionLiveEventsError> {
        let mut record_rx = SondaSessionTranscripts::subscribe_live(self, session_id)?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            while let Some(record) = record_rx.recv().await {
                if event_tx.send(record.event).is_err() {
                    break;
                }
            }
        });

        Ok(event_rx)
    }
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
                    content: content.into(),
                },
            },
        }
    }

    #[test]
    fn subscribe_replays_persisted_then_live_without_duplicates() {
        let dir = std::env::temp_dir().join(format!(
            "moray-subscribe-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("transcript.jsonl");
        jsonl::create_transcript_file(&path).expect("init transcript");
        let runtime = TranscriptRuntime::new(path);

        runtime.append(&wrap_user("a")).expect("append a");
        runtime.append(&wrap_user("b")).expect("append b");

        let mut rx = runtime.subscribe(1).expect("subscribe");

        let first = rx.try_recv().expect("replay seq 1");
        assert_eq!(first.seq, 1);
        let second = rx.try_recv().expect("replay seq 2");
        assert_eq!(second.seq, 2);
        assert!(rx.try_recv().is_err(), "no live events yet");

        runtime.append(&wrap_user("c")).expect("append c");
        let third = rx.try_recv().expect("live seq 3");
        assert_eq!(third.seq, 3);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn subscribe_live_skips_persisted_replay() {
        let dir = std::env::temp_dir().join(format!(
            "moray-subscribe-live-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("transcript.jsonl");
        jsonl::create_transcript_file(&path).expect("init transcript");
        let runtime = TranscriptRuntime::new(path);

        runtime.append(&wrap_user("a")).expect("append a");
        runtime.append(&wrap_user("b")).expect("append b");

        let mut rx = runtime.subscribe_live();
        assert!(rx.try_recv().is_err(), "no replay on live subscribe");

        runtime.append(&wrap_user("c")).expect("append c");
        let live = rx.try_recv().expect("live seq 3");
        assert_eq!(live.seq, 3);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reset_notifies_live_subscribers_and_resumes_seq() {
        let dir = std::env::temp_dir().join(format!(
            "moray-reset-live-sub-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("transcript.jsonl");
        jsonl::create_transcript_file(&path).expect("init transcript");
        let runtime = TranscriptRuntime::new(path);

        runtime.append(&wrap_user("before")).expect("append before");
        let mut rx = runtime.subscribe_live();

        runtime
            .append(&SessionEvent {
                session_id: "test-session".into(),
                ts: 2,
                kind: SessionEventKind::Reset,
            })
            .expect("reset");

        let reset_ev = rx.try_recv().expect("reset delivered to live subscriber");
        assert!(matches!(reset_ev.event.kind, SessionEventKind::Reset));
        assert_eq!(reset_ev.seq, 1);

        runtime.append(&wrap_user("after")).expect("append after reset");
        let after = rx.try_recv().expect("post-reset event delivered");
        assert_eq!(after.seq, 2);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reset_replaces_persisted_history() {
        let dir = std::env::temp_dir().join(format!(
            "moray-reset-transcript-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("transcript.jsonl");
        jsonl::create_transcript_file(&path).expect("init transcript");
        let runtime = TranscriptRuntime::new(path.clone());

        runtime.append(&wrap_user("before")).expect("append before");
        runtime
            .append(&SessionEvent {
                session_id: "test-session".into(),
                ts: 2,
                kind: SessionEventKind::Reset,
            })
            .expect("reset");

        let records = jsonl::read_transcript(&path).expect("read transcript");
        assert_eq!(records.len(), 1);
        assert!(matches!(records[0].event.kind, SessionEventKind::Reset));
        assert_eq!(records[0].seq, 1);

        let _ = std::fs::remove_dir_all(dir);
    }
}
