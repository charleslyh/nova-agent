use std::sync::Arc;

use moray_core::{ChatCompletion, Toolbox};

use crate::Result;

pub trait Harness: Send + Sync {
    fn create_completion(&self, session_id: &str) -> Result<Arc<dyn ChatCompletion>>;

    fn create_toolbox(&self, session_id: &str) -> Result<Arc<Toolbox>>;
}
