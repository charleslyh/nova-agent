//! Template-based system prompt provider.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use nova_core::{ChatCompletionRequestMessage, NovaError, ToolManifest};

use crate::context::PreambleProvider;
use crate::preambles::PreambleSection;

/// Hard-coded system prompt template + [`PreambleProvider`] implementation.
pub struct TemplatedPreambler {
    template: String,
    subs: BTreeMap<String, String>,
    subs_dyn: BTreeMap<String, Mutex<Box<dyn FnMut() -> String + Send>>>,
    sections: Vec<Box<dyn PreambleSection>>,
}

impl std::fmt::Debug for TemplatedPreambler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplatedPreambler")
            .field("template_len", &self.template.len())
            .field("sub_count", &self.subs.len())
            .field("sub_dyn_count", &self.subs_dyn.len())
            .field("section_count", &self.sections.len())
            .finish()
    }
}

#[derive(Default)]
pub struct TemplatedPreamblerBuilder {
    template: Option<String>,
    subs: BTreeMap<String, String>,
    subs_dyn: BTreeMap<String, Mutex<Box<dyn FnMut() -> String + Send>>>,
    sections: Vec<Box<dyn PreambleSection>>,
}

impl TemplatedPreamblerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    /// Bind `{{key}}` to a static value.
    pub fn subst(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.subs.insert(key.into(), value.into());
        self
    }

    /// Bind `{{key}}` to a closure; re-evaluated on each render.
    pub fn subst_dyn<F>(mut self, key: impl Into<String>, f: F) -> Self
    where
        F: FnMut() -> String + Send + 'static,
    {
        self.subs_dyn.insert(key.into(), Mutex::new(Box::new(f)));
        self
    }

    pub fn section(mut self, section: impl PreambleSection + 'static) -> Self {
        self.sections.push(Box::new(section));
        self
    }

    pub fn build(self) -> TemplatedPreambler {
        TemplatedPreambler {
            template: self.template.unwrap_or_default(),
            subs: self.subs,
            subs_dyn: self.subs_dyn,
            sections: self.sections,
        }
    }
}

impl TemplatedPreambler {
    /// Resolve template placeholders and sections (used by [`PreambleProvider::generate`]).
    pub fn render(&self) -> String {
        let mut preamble = self.template.clone();

        let keys = self
            .subs
            .keys()
            .chain(self.subs_dyn.keys())
            .collect::<BTreeSet<_>>();

        for key in keys {
            let value = if let Some(f) = self.subs_dyn.get(key) {
                match f.lock() {
                    Ok(mut f) => f(),
                    Err(_) => String::new(),
                }
            } else {
                self.subs.get(key).cloned().unwrap_or_default()
            };
            Self::replace(&mut preamble, key, &value);
        }

        Self::strip_unreplaced_placeholders(&mut preamble);

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

    fn strip_unreplaced_placeholders(preamble: &mut String) {
        while let Some(start) = preamble.find("{{") {
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
    ) -> Result<String, NovaError> {
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
                .subst("a", "1")
                .build(),
        );
        assert_eq!(text, "1 x ");
    }

    #[test]
    fn render_substitutes_arbitrary_key() {
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template("hello {{name}}")
                .subst("name", "world")
                .build(),
        );
        assert_eq!(text, "hello world");
    }

    #[test]
    fn render_substitutes_character() {
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template(TEST_TEMPLATE)
                .subst("character", "Mika.")
                .build(),
        );
        assert!(text.contains("Mika."));
        assert!(!text.contains("{{character}}"));
    }

    #[test]
    fn subst_dyn_invoked_each_generate() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let calls_in_fn = calls.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template("{{tag}}")
            .subst_dyn("tag", move || {
                calls_in_fn.fetch_add(1, Ordering::SeqCst);
                "live".into()
            })
            .build();
        rendered(&provider);
        rendered(&provider);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn character_dyn_invoked_each_generate() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let calls_in_fn = calls.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .subst_dyn("character", move || {
                calls_in_fn.fetch_add(1, Ordering::SeqCst);
                "from-fn".into()
            })
            .build();
        rendered(&provider);
        rendered(&provider);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn character_dyn_mut_can_change_value() {
        let state = std::sync::Arc::new(Mutex::new(1_u32));
        let state_in_fn = state.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template("n={{character}}")
            .subst_dyn("character", move || state_in_fn.lock().unwrap().to_string())
            .build();
        assert!(rendered(&provider).contains("n=1"));
        *state.lock().unwrap() = 2;
        assert!(rendered(&provider).contains("n=2"));
    }

    #[test]
    fn subst_dyn_overrides_subst_for_same_key() {
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template("{{key}}")
                .subst("key", "static")
                .subst_dyn("key", || "dynamic".into())
                .build(),
        );
        assert_eq!(text, "dynamic");
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
                .subst("character", "Mika.")
                .section(StaticSection("## Extra\n\nMore text."))
                .build(),
        );
        assert!(text.contains("## Character"));
        assert!(text.contains("Mika."));
        assert!(text.contains("## Extra"));
        assert!(text.contains("More text."));
    }

    #[test]
    fn character_dyn_empty_replaces_placeholder() {
        let text = rendered(
            &TemplatedPreamblerBuilder::new()
                .template(TEST_TEMPLATE)
                .subst_dyn("character", String::new)
                .build(),
        );
        assert!(text.contains("## Character"));
        assert!(!text.contains("{{character}}"));
    }

    #[test]
    fn generate_substitutes_character() {
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .subst("character", "test")
            .build();
        let content = provider.generate(&[], &[]).expect("generate");
        assert!(content.contains("## Character"));
        assert!(content.contains("test"));
    }

    #[test]
    fn generate_invokes_character_dyn_once_per_call() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let calls_in_fn = calls.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .subst_dyn("character", move || {
                calls_in_fn.fetch_add(1, Ordering::SeqCst);
                "Frozen.".into()
            })
            .build();
        provider.generate(&[], &[]).expect("generate 1");
        provider.generate(&[], &[]).expect("generate 2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn generate_reads_character_dyn_at_call_time() {
        use std::sync::RwLock;

        let current = std::sync::Arc::new(RwLock::new("v1".to_string()));
        let current_in_fn = current.clone();
        let provider = TemplatedPreamblerBuilder::new()
            .template(TEST_TEMPLATE)
            .subst_dyn("character", move || {
                current_in_fn.read().expect("lock").clone()
            })
            .build();
        let first = provider.generate(&[], &[]).expect("generate 1");
        *current.write().expect("lock") = "v2".into();
        let second = provider.generate(&[], &[]).expect("generate 2");
        assert!(first.contains("v1"));
        assert!(second.contains("v2"));
    }
}
