//! Template-based system prompt provider.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use moray_core::{ChatCompletionRequestMessage, MorayError, ToolManifest};

use crate::context::PreambleProvider;
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

/// Hard-coded system prompt template + [`PreambleProvider`] implementation.
pub struct TemplatedPreambler {
    template: String,
    sub_values: BTreeMap<String, String>,
    sub_preds: BTreeMap<String, Arc<dyn PreambleKeyPred>>,
    sections: Vec<Arc<dyn PreambleSection>>,
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
        }
    }
}

impl TemplatedPreambler {
    /// Resolve template, dynamic preds, and sections (used by [`PreambleProvider::generate`]).
    pub fn render(&self) -> String {
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

impl PreambleProvider for TemplatedPreambler {
    fn generate(
        &self,
        _transcript: &[ChatCompletionRequestMessage],
        _tools: &[ToolManifest],
    ) -> Result<String, MorayError> {
        Ok(self.render())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::context::PreambleProvider;

    use super::*;

    const TEST_TEMPLATE: &str = "## Character\n\n{{character}}\n";

    fn rendered(provider: &TemplatedPreambler) -> String {
        provider.generate(&[], &[]).expect("generate")
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
                .with_string("character", "Mika.")
                .build(),
        );
        assert!(text.contains("Mika."));
        assert!(!text.contains("{{character}}"));
    }

    #[test]
    fn render_pred_invoked_each_generate() {
        let pred = Arc::new(CountingPred {
            calls: AtomicUsize::new(0),
            value: "dynamic".into(),
        });
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_pred("character", pred.clone())
            .build();
        rendered(&provider);
        rendered(&provider);
        assert_eq!(pred.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn with_fn_invoked_each_generate() {
        let pred = Arc::new(CountingPred {
            calls: AtomicUsize::new(0),
            value: "fn-value".into(),
        });
        let pred_in_fn = pred.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_fn("character", move || {
                pred_in_fn.resolve();
                "from-fn".into()
            })
            .build();
        rendered(&provider);
        rendered(&provider);
        assert_eq!(pred.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn with_fn_mut_can_change_value() {
        let state = Arc::new(Mutex::new(1_u32));
        let state_in_fn = state.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template("n={{character}}")
            .with_fn("character", move || state_in_fn.lock().unwrap().to_string())
            .build();
        assert!(rendered(&provider).contains("n=1"));
        *state.lock().unwrap() = 2;
        assert!(rendered(&provider).contains("n=2"));
    }

    #[test]
    fn render_appends_sections_after_template() {
        use crate::preambles::PreambleSection;

        struct StaticSection(&'static str);
        impl PreambleSection for StaticSection {
            fn render(&self) -> String {
                self.0.into()
            }
        }

        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template("## Character\n\n{{character}}\n")
                .with_string("character", "Mika.")
                .section(StaticSection("## Extra\n\nMore text."))
                .build(),
        );
        assert!(text.contains("## Character"));
        assert!(text.contains("Mika."));
        assert!(text.contains("## Extra"));
        assert!(text.contains("More text."));
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
    fn generate_substitutes_static_character() {
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_string("character", "test")
            .build();
        let content = provider.generate(&[], &[]).expect("generate");
        assert!(content.contains("## Character"));
        assert!(content.contains("test"));
    }

    #[test]
    fn generate_invokes_dynamic_preds_once_per_call() {
        let pred = Arc::new(CountingPred {
            calls: AtomicUsize::new(0),
            value: "Frozen.".into(),
        });
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_pred("character", pred.clone())
            .build();
        provider.generate(&[], &[]).expect("generate 1");
        provider.generate(&[], &[]).expect("generate 2");
        assert_eq!(pred.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn generate_reads_pred_state_at_call_time() {
        use std::sync::RwLock;

        let current = Arc::new(RwLock::new("v1".to_string()));
        let current_in_fn = current.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .with_fn("character", move || current_in_fn.read().expect("lock").clone())
            .build();
        let first = provider.generate(&[], &[]).expect("generate 1");
        *current.write().expect("lock") = "v2".into();
        let second = provider.generate(&[], &[]).expect("generate 2");
        assert!(first.contains("v1"));
        assert!(second.contains("v2"));
    }
}
