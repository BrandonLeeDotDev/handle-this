//! Unified code generation utilities.
//!
//! This module consolidates code generation patterns that were previously
//! duplicated across checks.rs, error_handler.rs, and signal_handler.rs.
//!
//! ## Architecture
//!
//! - `action` - Action code generation (ReturnOk, Transform, Execute, ReturnDirect)
//! - `guard` - Guard wrapping (when conditions, match expressions)
//! - `check` - Type checking and binding generation

pub mod action;
pub mod check;
pub mod guard;
