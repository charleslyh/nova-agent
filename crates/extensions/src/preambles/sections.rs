//! Composable preamble sections appended after template substitution.

/// One fragment of the system prompt, rendered after the base template.
pub trait PreambleSection: Send + Sync {
    fn render(&self) -> String;
}
