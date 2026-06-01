//! Shared helpers for channel catalog secret merge/redact.

pub(crate) fn mask_secret(s: &str) -> String {
    if s.len() <= 4 {
        return "****".to_string();
    }
    format!("{}****", &s[..4.min(s.len())])
}

/// Keep the on-disk secret when the client resubmits the redacted placeholder or leaves it blank.
pub(crate) fn merge_preserved_secret(incoming: &str, existing: &str) -> String {
    let incoming = incoming.trim();
    if incoming.is_empty() {
        return existing.to_string();
    }
    if mask_secret(existing) == incoming {
        return existing.to_string();
    }
    incoming.to_string()
}
