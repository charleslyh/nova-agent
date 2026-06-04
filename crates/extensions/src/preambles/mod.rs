mod sections;
mod templated;

pub use sections::PreambleSection;
pub use templated::{PreambleKeyPred, TemplatedPreambler, TemplatedPreamblerBuilder};

pub use crate::context::PreambleProvider;
pub use crate::skills::SkillsSection;
