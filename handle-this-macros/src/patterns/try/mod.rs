//! Try patterns - various control flow structures with error handling.
//!
//! All try patterns share:
//! - Handler parsing (catch, throw, inspect, finally, with)
//! - Error handler generation
//! - Check generators for typed/catchall handlers
//! - Signal-based control flow for loop patterns

pub mod checks;
pub mod chain_builder;
pub mod common;
pub mod error_handler;
pub mod handlers;
pub mod signal;
pub mod signal_handler;

pub mod sync;
pub mod async_impl;
pub mod iter;
pub mod retry;
pub mod cond;
