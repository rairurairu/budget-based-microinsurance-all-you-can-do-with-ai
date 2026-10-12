//! Prompts are files in crates/ai/prompts, read at runtime so they change without recompiling.
//! After any prompt change run `just eval`.
use std::path::{Path, PathBuf};

pub fn load(dir: &Path, name: &str) -> std::io::Result<String> {
    std::fs::read_to_string(dir.join(format!("{name}.md")))
}

/// The prompt directory a process reads.
#[derive(Debug, Clone)]
pub struct Prompts {
    pub dir: PathBuf,
}

impl Prompts {
    pub fn new(dir: impl Into<PathBuf>) -> Prompts {
        Prompts { dir: dir.into() }
    }

    /// `crates/ai/prompts` under the repository root.
    pub fn in_repo(root: &Path) -> Prompts {
        Prompts::new(root.join("crates").join("ai").join("prompts"))
    }

    pub fn load(&self, component: &str) -> std::io::Result<String> {
        load(&self.dir, component)
    }
}
