//! Common types shared across try patterns.
//!
//! This module provides the unified `Handler` enum used by sync, async, iter, and retry patterns.
//! Having a single source of truth for handler types prevents duplication and ensures
//! consistent behavior across all try pattern variants.

use crate::keywords::catch::CatchClause;
use crate::keywords::throw::ThrowClause;
use crate::keywords::inspect::InspectClause;
use crate::keywords::try_catch::TryCatchClause;

/// A handler in declaration order.
///
/// This enum tracks the type and position of each handler in the handler chain.
/// Handlers are processed in declaration order to ensure predictable behavior.
#[derive(Clone)]
pub enum Handler {
    /// `catch [Type(binding)] [guard] { body }` - catches error, returns Ok(body)
    Catch(CatchClause),
    /// `throw [Type(binding)] [guard] { expr }` - transforms error, continues chain
    Throw(ThrowClause),
    /// `inspect [Type(binding)] [guard] { body }` - runs side effect, error propagates
    Inspect(InspectClause),
    /// `try catch [Type(binding)] [guard] { body }` - body returns Result directly
    TryCatch(TryCatchClause),
}

impl Handler {
    /// Returns the name of this handler type (for error messages).
    pub fn name(&self) -> &'static str {
        match self {
            Handler::Catch(_) => "catch",
            Handler::Throw(_) => "throw",
            Handler::Inspect(_) => "inspect",
            Handler::TryCatch(_) => "try catch",
        }
    }

    /// Returns true if this is an untyped catch or try catch (catches all errors).
    pub fn is_untyped_catchall(&self) -> bool {
        match self {
            Handler::Catch(c) => c.type_path.is_none() && c.guard.is_none(),
            Handler::TryCatch(tc) => tc.type_path.is_none() && tc.guard.is_none(),
            _ => false,
        }
    }
}
