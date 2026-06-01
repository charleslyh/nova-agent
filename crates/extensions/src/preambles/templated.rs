//! Template-based system prompt pipeline node.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use moray_core::{ChatCompletionRequestMessage, MorayError, ToolManifest};

use crate::context::ContextPipelineNode;
use crate::preambles::PreambleSection;

/// Resolves one template placeholder at render time (`{{key}}`).
pub trait PreambleKeyPred: Send + Sync {
    fn resolve(&self) -> String;
}

/// Bridges a `FnMut` closure into [`PreambleKeyPred`].
struct PreambleSubstitutionFn {
    inner: Mutex<Box<dyn FnMut() -> String + Send>>,
}

impl PreambleSubstitutionFn {
    fn new<F>(f: F) -> Self
    where
        F: FnMut() -> String + Send + 'static,
    {
        Self {
            inner: Mutex::new(Box::new(f)),
        }
    }
}

impl PreambleKeyPred for PreambleSubstitutionFn {
    fn resolve(&self) -> String {
        match self.inner.lock() {
            Ok(mut f) => f(),
            Err(_) => String::new(),
        }
    }
}

/// Hard-coded system prompt template + [`ContextPipelineNode`] implementation.
pub struct TemplatedPreambler {
    template: String,
    sub_values: BTreeMap<String, String>,
    sub_preds: BTreeMap<String, Arc<dyn PreambleKeyPred>>,
    sections: Vec<Arc<dyn PreambleSection>>,
    /// Preamble frozen for the current agent run; set in [`ContextPipelineNode::bootstrap`].
    turn_preamble: RwLock<Option<String>>,
}

impl std::fmt::Debug for TemplatedPreambler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplatedPreambler")
            .field("template_len", &self.template.len())
            .field("values", &self.sub_values)
            .field("pred_count", &self.sub_preds.len())
            .field("section_count", &self.sections.len())
            .finish()
    }
}

#[derive(Default)]
pub struct TemplatedPreamblerBuilder {
    template: Option<String>,
    values: BTreeMap<String, String>,
    preds: BTreeMap<String, Arc<dyn PreambleKeyPred>>,
    sections: Vec<Arc<dyn PreambleSection>>,
}

impl TemplatedPreamblerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    pub fn with_string(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }

    pub fn with_pred(
        mut self,
        key: impl Into<String>,
        pred: Arc<dyn PreambleKeyPred>,
    ) -> Self {
        self.preds.insert(key.into(), pred);
        self
    }

    pub fn with_fn<F>(mut self, key: impl Into<String>, f: F) -> Self
    where
        F: FnMut() -> String + Send + 'static,
    {
        self.preds
            .insert(key.into(), Arc::new(PreambleSubstitutionFn::new(f)));
        self
    }

    pub fn section(mut self, section: impl PreambleSection + 'static) -> Self {
        self.sections.push(Arc::new(section));
        self
    }

    pub fn build(self) -> TemplatedPreambler {
        TemplatedPreambler {
            template: self.template.unwrap_or_default(),
            sub_values: self.values,
            sub_preds: self.preds,
            sections: self.sections,
            turn_preamble: RwLock::new(None),
        }
    }
}

fn preamble_lock_err() -> MorayError {
    MorayError::Message("TemplatedPreambler turn_preamble lock poisoned".into())
}

impl TemplatedPreambler {
    fn turn_preamble(&self) -> Result<String, MorayError> {
        self.turn_preamble
            .read()
            .map_err(|_| preamble_lock_err())?
            .clone()
            .ok_or_else(|| {
                MorayError::Message(
                    "TemplatedPreambler: bootstrap required before process".into(),
                )
            })
    }

    /// Resolve template, dynamic preds, and sections once per agent run.
    fn render_once(&self) -> String {
        let mut preamble = self.template.clone();

        for (key, value) in &self.sub_values {
            Self::replace(&mut preamble, key, value);
        }

        for (key, pred) in &self.sub_preds {
            Self::replace(&mut preamble, key, &pred.resolve());
        }

        Self::strip_unreplaced_needles(&mut preamble);

        for section in &self.sections {
            let part = section.render();
            if part.trim().is_empty() {
                continue;
            }
            if !preamble.is_empty() {
                preamble.push_str("\n\n");
            }
            preamble.push_str(part.trim_end());
        }

        preamble
    }

    #[inline]
    fn replace(preamble: &mut String, key: &str, value: &str) {
        let needle = format!("{{{{{key}}}}}");
        *preamble = preamble.replace(&needle, value);
    }

    fn strip_unreplaced_needles(preamble: &mut String) {
        loop {
            let Some(start) = preamble.find("{{") else {
                break;
            };
            let after = start + 2;
            let Some(end_rel) = preamble[after..].find("}}") else {
                break;
            };
            let end = after + end_rel + 2;
            preamble.replace_range(start..end, "");
        }
    }
}

impl ContextPipelineNode for TemplatedPreambler {
    fn bootstrap(&self) -> Result<(), MorayError> {
        *self
            .turn_preamble
            .write()
            .map_err(|_| preamble_lock_err())? = Some(self.render_once());
        Ok(())
    }

    fn process(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        _tools: &[ToolManifest],
    ) -> Result<Vec<ChatCompletionRequestMessage>, MorayError> {
        let sysmsg = ChatCompletionRequestMessage::System {
            content: self.turn_preamble()?,
        };

        let mut messages = messages;
        if matches!(messages.first(), Some(ChatCompletionRequestMessage::System { .. })) {
            messages[0] = sysmsg;
        } else {
            messages.insert(0, sysmsg);
        }

        Ok(messages)
    }

    fn teardown(&self) -> Result<(), MorayError> {
        *self
            .turn_preamble
            .write()
            .map_err(|_| preamble_lock_err())? = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const TEST_TEMPLATE: &str = "## Character\n\n{{character}}\n";

    fn rendered(node: &TemplatedPreambler) -> String {
        node.render_once()
    }

    struct CountingPred {
        calls: AtomicUsize,
        value: String,
    }

    impl PreambleKeyPred for CountingPred {
        fn resolve(&self) -> String {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.value.clone()
        }
    }

    #[test]
    fn render_strips_unreplaced_placeholders() {
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template(TEST_TEMPLATE)
                .build(),
        );
        assert!(text.contains("## Character"));
        assert!(!text.contains("{{character}}"));
    }

    #[test]
    fn render_strips_multiple_unreplaced_placeholders() {
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template("{{a}} x {{b}}")
                .with_string("a", "1")
                .build(),
        );
        assert_eq!(text, "1 x ");
    }

    #[test]
    fn render_substitutes_static_character() {
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template(TEST_TEMPLATE)
                .with_string("character", "A meticulous reviewer.")
                .build(),
        );
        assert!(text.contains("A meticulous reviewer."));
    }

    #[test]
    fn render_appends_sections_after_template() {
        use crate::preambles::PreambleSection;

        struct StaticSection(&'static str);
        impl PreambleSection for StaticSection {
            fn render(&self) -> String {
                self.0.to_string()
            }
        }

        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template("## Character\n\n{{character}}\n")
                .with_string("character", "Hi")
                .section(StaticSection("## Skills\n\n<available_skills/>"))
                .build(),
        );
        assert!(text.contains("Hi"));
        assert!(text.contains("<available_skills/>"));
        assert!(text.find("Hi").unwrap() < text.find("<available_skills").unwrap());
    }

    #[test]
    fn render_pred_invoked_each_render() {
        let pred = Arc::new(CountingPred {
            calls: AtomicUsize::new(0),
            value: "From pred.".into(),
        });
        let node = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_pred("character", pred.clone())
            .build();
        let a = rendered(&node);
        let b = rendered(&node);
        assert!(a.contains("From pred."));
        assert_eq!(pred.calls.load(Ordering::SeqCst), 2);
        assert_eq!(a, b);
    }

    #[test]
    fn with_fn_invoked_each_render() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_fn = calls.clone();
        let node = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_fn(
                "character",
                move || {
                    calls_in_fn.fetch_add(1, Ordering::SeqCst);
                    "From fn.".into()
                },
            )
            .build();
        rendered(&node);
        rendered(&node);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn with_fn_mut_can_change_value() {
        let counter = AtomicUsize::new(0);
        let node = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_fn(
                "character",
                move || {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    format!("value-{n}")
                },
            )
            .build();
        assert!(rendered(&node).contains("value-0"));
        assert!(rendered(&node).contains("value-1"));
    }

    #[test]
    fn render_pred_empty_replaces_placeholder() {
        struct EmptyPred;
        impl PreambleKeyPred for EmptyPred {
            fn resolve(&self) -> String {
                String::new()
            }
        }
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template(TEST_TEMPLATE)
                .with_pred("character", Arc::new(EmptyPred))
                .build(),
        );
        assert!(text.contains("## Character"));
        assert!(!text.contains("{{character}}"));
    }

    #[test]
    fn process_prepends_system() {
        let node = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_string("character", "test")
            .build();
        node.bootstrap().expect("bootstrap");
        let out = node
            .process(
                vec![ChatCompletionRequestMessage::User {
                    content: "hi".into(),
                }],
                &[],
            )
            .expect("process");
        assert!(matches!(
            out.first(),
            Some(ChatCompletionRequestMessage::System { .. })
        ));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn bootstrap_freezes_dynamic_preds_for_process() {
        let pred = Arc::new(CountingPred {
            calls: AtomicUsize::new(0),
            value: "Frozen.".into(),
        });
        let node = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_pred("character", pred.clone())
            .build();
        node.bootstrap().expect("bootstrap");
        assert_eq!(pred.calls.load(Ordering::SeqCst), 1);

        let _ = node.process(vec![], &[]).expect("process 1");
        let _ = node.process(vec![], &[]).expect("process 2");
        assert_eq!(pred.calls.load(Ordering::SeqCst), 1);

        node.teardown().expect("teardown");
        rendered(&node);
        assert_eq!(pred.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn process_without_bootstrap_errors() {
        let node = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_string("character", "test")
            .build();
        let err = node.process(vec![], &[]).expect_err("process");
        assert!(
            err.to_string()
                .contains("bootstrap required before process")
        );
    }

    #[test]
    fn bootstrap_snapshot_ignores_later_pred_changes_until_next_bootstrap() {
        let current = Arc::new(RwLock::new("v1".to_string()));
        let current_in_fn = current.clone();
        let node = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_fn("character", move || current_in_fn.read().expect("lock").clone())
            .build();
        node.bootstrap().expect("bootstrap");
        *current.write().expect("lock") = "v2".into();
        let out = node.process(vec![], &[]).expect("process");
        let ChatCompletionRequestMessage::System { content } = &out[0] else {
            panic!("expected system message");
        };
        assert!(content.contains("v1"));
        assert!(!content.contains("v2"));
    }
}
