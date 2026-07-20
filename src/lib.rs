pub mod clipboard;
pub mod config;
pub mod extract;
pub mod extract_app;
pub mod extract_ui;
pub mod herdr_client;
pub mod theme;

/// What extractor input produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Continue,
    Copy(String),
    Cancel,
}
