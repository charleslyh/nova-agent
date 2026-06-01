//! In-memory snapshot: session catalog view + per-session work status.
//!
//! Read path: [`SondaSnapshot::subscribe`] delivers a snapshot then live deltas.
//! Write path: catalog mutations and session events.

use std::sync::Mutex;

use moray_core::MorayError;
use moray_session::{SessionEvent, SessionEventKind, SessionEventSink};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::session_catalog::{SessionCatalogEntry, SondaSessionCatalog};

/// Per-session turn activity for sidebar / multi-client sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionWorkStatus {
    Idle,
    Running,
}

/// One row in the Sonda session list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SondaSessionListEntry {
    pub session_id: String,
    pub name: String,
    pub work_status: SessionWorkStatus,
}

/// Sonda state events streamed to subscribers (SSE `event: sonda`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SondaStateEvent {
    Snapshot {
        revision: u64,
        sessions: Vec<SondaSessionListEntry>,
    },
    SessionAdded {
        revision: u64,
        session: SondaSessionListEntry,
    },
    SessionRemoved {
        revision: u64,
        session_id: String,
    },
    WorkStatusChanged {
        revision: u64,
        session_id: String,
        work_status: SessionWorkStatus,
    },
}

impl SondaStateEvent {
    pub fn revision(&self) -> u64 {
        match self {
            Self::Snapshot { revision, .. }
            | Self::SessionAdded { revision, .. }
            | Self::SessionRemoved { revision, .. }
            | Self::WorkStatusChanged { revision, .. } => *revision,
        }
    }
}

#[derive(Debug)]
struct Subscriber {
    start_revision: u64,
    tx: mpsc::UnboundedSender<SondaStateEvent>,
}

#[derive(Debug)]
struct SondaStateInner {
    revision: u64,
    sessions: Vec<SondaSessionListEntry>,
    subscribers: Vec<Subscriber>,
}

/// Fan-out store for session list and work status.
pub struct SondaSnapshot {
    inner: Mutex<SondaStateInner>,
}

impl SondaSnapshot {
    pub fn new(catalog: &SondaSessionCatalog) -> Self {
        let sessions = catalog
            .entries()
            .into_iter()
            .map(entry_from_catalog)
            .collect();
        Self {
            inner: Mutex::new(SondaStateInner {
                revision: 0,
                sessions,
                subscribers: Vec::new(),
            }),
        }
    }

    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<SondaStateEvent> {
        let (tx, rx) = mpsc::unbounded_channel();

        let snapshot = {
            let mut inner = self.inner.lock().expect("sonda state lock poisoned");
            let start_revision = inner.revision.saturating_add(1);
            inner.subscribers.push(Subscriber {
                start_revision,
                tx: tx.clone(),
            });
            Self::snapshot_from(&inner)
        };

        let _ = tx.send(snapshot);
        rx
    }

    pub(crate) fn notify_session_added(&self, session_id: &str, name: &str) {
        let session = SondaSessionListEntry {
            session_id: session_id.to_string(),
            name: name.to_string(),
            work_status: SessionWorkStatus::Idle,
        };
        let mut inner = self.inner.lock().expect("sonda state lock poisoned");
        if inner
            .sessions
            .iter()
            .any(|e| e.session_id == session.session_id)
        {
            return;
        }
        inner.sessions.push(session.clone());
        let event = SondaStateEvent::SessionAdded {
            revision: Self::next_revision(&mut inner),
            session,
        };
        Self::publish_locked(&mut inner, event);
    }

    pub(crate) fn notify_session_removed(&self, session_id: &str) {
        let mut inner = self.inner.lock().expect("sonda state lock poisoned");
        let before = inner.sessions.len();
        inner
            .sessions
            .retain(|e| e.session_id != session_id);
        if inner.sessions.len() == before {
            return;
        }
        let event = SondaStateEvent::SessionRemoved {
            revision: Self::next_revision(&mut inner),
            session_id: session_id.to_string(),
        };
        Self::publish_locked(&mut inner, event);
    }

    fn set_work_status(&self, session_id: &str, work_status: SessionWorkStatus) {
        let mut inner = self.inner.lock().expect("sonda state lock poisoned");
        let Some(entry) = inner
            .sessions
            .iter_mut()
            .find(|e| e.session_id == session_id)
        else {
            return;
        };
        if entry.work_status == work_status {
            return;
        }
        entry.work_status = work_status;
        let event = SondaStateEvent::WorkStatusChanged {
            revision: Self::next_revision(&mut inner),
            session_id: session_id.to_string(),
            work_status,
        };
        Self::publish_locked(&mut inner, event);
    }

    fn apply_session_event(&self, event: &SessionEvent) {
        let work_status = match &event.kind {
            SessionEventKind::TurnAccepted { .. } => Some(SessionWorkStatus::Running),
            SessionEventKind::TurnFinish | SessionEventKind::Reset => Some(SessionWorkStatus::Idle),
            SessionEventKind::AgentResponse { .. } => None,
        };

        if let Some(status) = work_status {
            self.set_work_status(event.session_id.as_str(), status);
        }
    }

    fn snapshot_from(inner: &SondaStateInner) -> SondaStateEvent {
        SondaStateEvent::Snapshot {
            revision: inner.revision,
            sessions: inner.sessions.clone(),
        }
    }

    fn next_revision(inner: &mut SondaStateInner) -> u64 {
        inner.revision = inner.revision.saturating_add(1);
        inner.revision
    }

    fn publish_locked(inner: &mut SondaStateInner, event: SondaStateEvent) {
        let revision = event.revision();
        inner.subscribers.retain(|sub| {
            if revision < sub.start_revision {
                return true;
            }
            sub.tx.send(event.clone()).is_ok()
        });
    }
}

fn entry_from_catalog(entry: SessionCatalogEntry) -> SondaSessionListEntry {
    let name = if entry.name.is_empty() {
        entry.session_id.clone()
    } else {
        entry.name
    };
    SondaSessionListEntry {
        session_id: entry.session_id,
        name,
        work_status: SessionWorkStatus::Idle,
    }
}

impl SessionEventSink for SondaSnapshot {
    fn append(&self, event: &SessionEvent) -> Result<(), MorayError> {
        self.apply_session_event(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_catalog() -> (tempfile::TempDir, SondaSessionCatalog) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.toml");
        std::fs::write(
            &path,
            r#"default_agent_id = "z9y8x7w6"

[[entries]]
session_id = "s1"
name = "One"
"#,
        )
        .expect("write sessions.toml");
        let catalog = SondaSessionCatalog::open(&path).expect("open catalog");
        (dir, catalog)
    }

    #[test]
    fn subscribe_receives_snapshot_first() {
        let (_dir, catalog) = temp_catalog();
        let snapshot = SondaSnapshot::new(&catalog);
        let mut rx = snapshot.subscribe();
        let ev = rx.try_recv().expect("snapshot");
        assert!(matches!(ev, SondaStateEvent::Snapshot { .. }));
        if let SondaStateEvent::Snapshot { sessions, .. } = ev {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].session_id, "s1");
        }
    }

    #[test]
    fn session_added_and_work_status_delta() {
        let (_dir, catalog) = temp_catalog();
        let snapshot = SondaSnapshot::new(&catalog);
        let mut rx = snapshot.subscribe();
        let _ = rx.try_recv().expect("snapshot");

        snapshot.notify_session_added("s2", "Two");
        let added = rx.try_recv().expect("added");
        assert!(matches!(added, SondaStateEvent::SessionAdded { .. }));

        snapshot.set_work_status("s2", SessionWorkStatus::Running);
        let running = rx.try_recv().expect("running");
        assert!(
            matches!(
                running,
                SondaStateEvent::WorkStatusChanged {
                    work_status: SessionWorkStatus::Running,
                    ..
                }
            )
        );

        snapshot.set_work_status("s2", SessionWorkStatus::Running);
        assert!(rx.try_recv().is_err(), "deduped status");
    }
}
