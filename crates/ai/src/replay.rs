//! Saved model replies, for demo mode and evaluations: the process never calls the network.
//! Each file in `fixtures/ai_replies/<component>/*.json` holds the reply text the model gave
//! (or is assumed to give) for one input, keyed by the component and the exact input sent (and
//! any images):
//!
//! ```json
//! { "key": "explain:1f3a…", "input": { … }, "reply": "{\"explanations\": …}" }
//! ```
//!
//! `input` is kept for people reading the file; only `key` and `reply` are used. A request with
//! no saved reply is an error, so the harness uses the saved fallback (R22). The key ignores the
//! prompt, so saved replies survive prompt edits; `just eval` with `EVAL_LIVE=1` checks the
//! prompts themselves.
use crate::client::{Model, ModelRequest};
use crate::harness::AiError;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// The replay key of a request: its component and a hash of its input and images.
pub fn key(req: &ModelRequest) -> String {
    let mut hash = Sha256::new();
    hash.update(req.user.as_bytes());
    for image in &req.images {
        hash.update(&image.bytes);
    }
    let hex: String = hash.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{}:{hex}", req.component)
}

#[derive(Deserialize)]
struct Saved {
    key: String,
    reply: String,
}

#[derive(Debug, Default)]
pub struct ReplayModel {
    replies: BTreeMap<String, String>,
}

impl ReplayModel {
    /// Reads every `*.json` under `dir` (one level of component folders).
    pub fn load(dir: &Path) -> Result<ReplayModel, String> {
        let mut replies = BTreeMap::new();
        if !dir.exists() {
            return Ok(ReplayModel { replies });
        }
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                for f in std::fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))? {
                    files.push(f.map_err(|e| e.to_string())?.path());
                }
            }
        }
        files.sort();
        for path in files
            .iter()
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
        {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let saved: Saved =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            if replies.insert(saved.key.clone(), saved.reply).is_some() {
                return Err(format!("{}: key {} saved twice", path.display(), saved.key));
            }
        }
        Ok(ReplayModel { replies })
    }

    pub fn insert(&mut self, key: String, reply: String) {
        self.replies.insert(key, reply);
    }

    pub fn len(&self) -> usize {
        self.replies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.replies.is_empty()
    }
}

impl Model for ReplayModel {
    async fn complete(&self, req: &ModelRequest) -> Result<String, AiError> {
        let k = key(req);
        let reply = self.replies.get(&k).cloned();
        if reply.is_none() {
            log_miss(&k, req);
        }
        reply.ok_or_else(|| AiError::Call(format!("no saved reply for {k}")))
    }
}

/// With `AI_REPLAY_MISSES=<file>`: each request with no saved reply is appended to the file as a
/// JSON line (its key, component and input), so a golden case can be written for it.
fn log_miss(key: &str, req: &ModelRequest) {
    let Ok(path) = std::env::var("AI_REPLAY_MISSES") else {
        return;
    };
    let line = serde_json::json!({ "key": key, "component": req.component, "input": req.user });
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes()));
    if let Err(e) = written {
        eprintln!("ai      AI_REPLAY_MISSES {path}: {e}");
    }
}
