//! Shared tool context: a per-run, key-value store shared across every tool call
//! in a single ReAct loop.
//!
//! [`ToolContext`] is owned by the [`Toolbox`](crate::toolbox::Toolbox) and passed to each
//! [`Tool::call`](crate::toolbox::Tool::call) / [`TypedTool::run`](crate::toolbox::TypedTool::run).
//! It lets a tool chain (tool1 -> tool2 -> ... -> toolN) hand data to its successors without
//! a third-party backend service. The agent/react layer is unaware of it.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;

/// Shared, clonable key-value context for a single agent run.
///
/// Backed by `Arc<RwLock<HashMap<String, Value>>>`: cloning is cheap (atomic refcount), and
/// concurrent tools in a [`ToolCallGroup`](crate::toolbox::ToolCallGroup) can read/write safely.
///
/// Tools should only access the context in synchronous code — never hold the write guard
/// across an `.await`, as that can stall other tools waiting on the same lock.
#[derive(Clone, Default)]
pub struct ToolContext {
    inner: Arc<RwLock<HashMap<String, Value>>>,
}

impl ToolContext {
    /// Creates an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a clone of the value stored under `key`, if present.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.inner.read().expect("tool context lock poisoned").get(key).cloned()
    }

    /// Inserts a value, returning the previous value under `key`, if any.
    pub fn insert(&self, key: impl Into<String>, value: Value) -> Option<Value> {
        self.inner
            .write()
            .expect("tool context lock poisoned")
            .insert(key.into(), value)
    }

    /// Removes and returns the value stored under `key`, if present.
    pub fn remove(&self, key: &str) -> Option<Value> {
        self.inner
            .write()
            .expect("tool context lock poisoned")
            .remove(key)
    }

    /// Returns `true` if the context contains a value for `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.inner.read().expect("tool context lock poisoned").contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn insert_get_remove_roundtrip() {
        let ctx = ToolContext::new();
        assert!(!ctx.contains_key("a"));
        assert_eq!(ctx.insert("a", json!(1)), None);
        assert!(ctx.contains_key("a"));
        assert_eq!(ctx.get("a"), Some(json!(1)));
        assert_eq!(ctx.insert("a", json!(2)), Some(json!(1)));
        assert_eq!(ctx.get("a"), Some(json!(2)));
        assert_eq!(ctx.remove("a"), Some(json!(2)));
        assert!(!ctx.contains_key("a"));
        assert_eq!(ctx.get("a"), None);
    }

    #[test]
    fn clones_share_state() {
        let a = ToolContext::new();
        let b = a.clone();
        a.insert("k", json!("v"));
        assert_eq!(b.get("k"), Some(json!("v")));
    }
}
