/// Why the engine refused an action. `rule` is an ID from docs/rules.md.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("rule {rule} blocked the action: {detail}")]
    Rule { rule: &'static str, detail: String },
    #[error("invalid configuration: {0}")]
    Config(String),
    /// A lifecycle step the record's current state does not allow, such as accepting a task
    /// twice.
    #[error("not allowed in this state: {0}")]
    State(String),
}
