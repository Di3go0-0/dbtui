//! SQL Engine — semantic analysis layer for completion, diagnostics, and validation.
//!
//! Sits between core (adapter trait, models) and UI (rendering, events).
//! Pure SQL analysis: no UI imports, no database I/O.

// Phase A foundation: consumers come in Phases B-F.
#[allow(dead_code)]
pub mod analyzer;
#[allow(dead_code)]
pub mod completion;
#[allow(dead_code)]
pub mod context;
#[allow(dead_code)]
pub mod diagnostics;
#[allow(dead_code)]
pub mod dialect;
#[allow(dead_code)]
pub mod metadata;
#[allow(dead_code)]
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
