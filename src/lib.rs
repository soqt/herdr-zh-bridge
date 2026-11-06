//! Herdr ZH Bridge: a Chinese ↔ English companion for existing LLM CLI
//! sessions in Herdr. `core` is the UI-free engine; `ui` is the GPUI app.

pub mod core;
pub mod ui;

#[derive(Debug, Default)]
pub struct Options {
    /// Bind or focus a tab for this Herdr terminal address.
    pub target: String,
    /// Start a separate app process and return immediately (for Herdr's exec).
    pub detach: bool,
    /// Demo data, no CLI calls, temporary preferences.
    pub preview: bool,
    /// Build the workspace, then quit.
    pub smoke_test: bool,
}
