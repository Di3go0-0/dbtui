//! SQL Engine — semantic analysis layer for completion, diagnostics, and validation.
//!
//! Sits between core (adapter trait, models) and UI (rendering, events).
//! Pure SQL analysis: no UI imports, no database I/O.

pub mod analyzer;
pub mod completion;
pub mod context;
pub mod diagnostics;
pub mod dialect;
pub mod metadata;
pub mod models;
pub mod tokenizer;

use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug, Clone)]
pub enum EngineError {
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Resolution error: {0}")]
    Resolution(String),
}

#[allow(dead_code)]
pub type EngineResult<T> = Result<T, EngineError>;
