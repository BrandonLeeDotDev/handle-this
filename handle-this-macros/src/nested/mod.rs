//! Recursive transformation of nested handle! patterns.
//!
//! This module scans token streams for nested try/catch/throw/inspect patterns
//! and transforms them into the appropriate generated code, allowing users to
//! write nested error handling without explicit handle! invocations.
//!
//! ## Module Structure
//!
//! - `detection` - Detection utilities (contains_control_flow, contains_question_mark, etc.)
//! - `transform` - Main transformation logic and pattern handlers
//!
//! ## Supported nested patterns
//!
//! - `try { } catch/throw/inspect/finally ...`
//! - `try while COND { } ...`
//! - `try for PAT in ITER { } ...`
//! - `try any PAT in ITER { } ...`
//! - `try all PAT in ITER { } ...`
//! - `try when COND { } else when COND { } else { } ...`
//! - `scope "name", ...`
//! - `require COND else "msg", ...`

mod detection;
mod transform;

// Re-export public API
pub use detection::{contains_control_flow, contains_question_mark};
pub use transform::transform_nested;
