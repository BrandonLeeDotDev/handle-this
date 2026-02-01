//! Declarative macros for handle-this.

mod helpers;

// The handle! macro is defined here with #[macro_export], which exports it at crate root
#[macro_use]
mod handle;

pub use helpers::*;
