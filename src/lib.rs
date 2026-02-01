//! handle-this - Ergonomic error handling with try/catch/throw/inspect/finally
//!
//! # Overview
//!
//! `handle-this` provides composable error handling with automatic stack traces.
//! All invocations return `Result<T>` - no hidden control flow.
//!
//! # Quick Start
//!
//! ```
//! use handle_this::{handle, Result};
//!
//! fn load_data(path: &str) -> Result<String> {
//!     handle!{ try { std::fs::read_to_string(path)? } with "reading" }
//! }
//! ```
//!
//! # Patterns
//!
//! ## Basic
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | `try { }` | Execute, wrap error with trace |
//! | `try { } catch e { }` | Recover from error |
//! | `try { } catch Type(e) { }` | Recover only specific type |
//! | `try { } catch Type(e) { } else { }` | Typed catch with fallback |
//! | `try { } try catch e { }` | Fallible recovery (body returns Result) |
//! | `try { } throw e { }` | Transform error |
//! | `try { } throw Type(e) { }` | Transform only specific type |
//! | `try { } inspect e { }` | Side effect, then propagate |
//! | `try { } finally { }` | Cleanup always runs |
//! | `try -> T { } else { }` | Infallible (returns T, not Result) |
//!
//! ## Guards
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | `catch e when cond { }` | Conditional catch |
//! | `catch Type(e) when cond { }` | Typed with guard |
//! | `throw e when cond { }` | Conditional transform |
//! | `catch Type(e) match expr { arms }` | Match on error value |
//!
//! ## Chain Search
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | `catch any Type(e) { }` | First matching error in cause chain |
//! | `catch all Type \|errs\| { }` | All matching errors as Vec |
//!
//! ## Context
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | `try { } with "message"` | Add context message |
//! | `try { } with { key: val }` | Add structured data |
//! | `try { } with "msg", { key: val }` | Both message and data |
//! | `scope "name", try { }` | Hierarchical scope |
//! | `require cond else "msg", try { }` | Precondition check |
//!
//! ## Chaining
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | `try { a()? }, then \|x\| { b(x)? }` | Chain operations |
//!
//! ## Iteration
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | `try for x in iter { }` | First success |
//! | `try any x in iter { }` | Alias for try for |
//! | `try all x in iter { }` | Collect all results |
//! | `try while cond { }` | Retry loop |
//!
//! ## Async
//!
//! | Pattern | Description |
//! |---------|-------------|
//! | `async try { }` | Async version (all patterns supported) |

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

// ============================================================
// Modules
// ============================================================

mod handled;
mod ext;
mod macros;

// ============================================================
// Re-exports
// ============================================================

pub use handled::{Handled, FrameView, Error, StringError, TryCatch, Value, IntoValue};
pub use ext::HandleExt;

// Internal helper for macros
#[doc(hidden)]
#[cfg(feature = "std")]
pub use handled::__wrap_any;

// Re-export proc-macro crate for nested pattern expansion
#[doc(hidden)]
pub use handle_this_macros;

// ============================================================
// Type aliases
// ============================================================

/// Result type alias.
///
/// - `Result<T>` = `core::result::Result<T, Handled>` (type-erased)
/// - `Result<T, Handled<io::Error>>` = preserves concrete error type
pub type Result<T, E = Handled> = core::result::Result<T, E>;

/// Result module for `try catch` blocks.
#[allow(non_snake_case)]
pub mod result {
    use super::Handled;

    /// Result type alias for `try catch` blocks.
    pub type Result<T> = core::result::Result<T, Handled>;

    /// Create an `Ok` result.
    #[inline]
    pub fn Ok<T>(v: T) -> Result<T> {
        core::result::Result::Ok(v)
    }

    /// Create an `Err` result with automatic conversion to `Handled`.
    #[inline]
    pub fn Err<T>(e: impl Into<Handled>) -> Result<T> {
        core::result::Result::Err(e.into())
    }
}

/// Type alias for errors in chain closures.
#[doc(hidden)]
#[cfg(feature = "std")]
pub type __BoxedError = Box<dyn std::error::Error + Send + Sync + 'static>;

// Re-export helper functions for macros
#[doc(hidden)]
pub use macros::{
    __map_try_erased, __with_finally, __wrap_frame,
    __ThrowExpr, __Thrown,
    __convert_try_catch_result, __convert_try_catch_result_str,
    __ErrWrap, __IntoHandled,
    TryCatchConvert, TryCatchResult,
};

// ============================================================
// Loop Signal - Control Flow as Data
// ============================================================

/// Internal signal type for control flow in loop patterns.
///
/// When handlers contain `continue` or `break`, the macro transforms them into
/// signal values that get translated to actual control flow at the expansion site.
///
/// # Why This Exists
///
/// Rust prevents control flow from escaping closures. This type allows:
/// 1. Handlers to "request" control flow via return values
/// 2. Typed catches to propagate unmatched errors (instead of `unreachable!()`)
/// 3. Signals to compose through nested try blocks
///
/// # Safety
///
/// This type is `#[doc(hidden)]` and only used by macro-generated code.
/// All variants are exhaustively matched - no signals are ever dropped.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum __LoopSignal<T> {
    /// Normal completion with a value.
    Value(T),
    /// Signal to execute `continue` on the target loop.
    Continue,
    /// Signal to execute `break` on the target loop.
    Break,
}

impl<T> __LoopSignal<T> {
    /// Map the inner value, preserving control flow signals.
    #[inline]
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> __LoopSignal<U> {
        match self {
            __LoopSignal::Value(v) => __LoopSignal::Value(f(v)),
            __LoopSignal::Continue => __LoopSignal::Continue,
            __LoopSignal::Break => __LoopSignal::Break,
        }
    }

    /// Check if this is a control flow signal (not a value).
    #[inline]
    pub fn is_control_flow(&self) -> bool {
        !matches!(self, __LoopSignal::Value(_))
    }

    /// Extract the value, panicking on control flow signals.
    ///
    /// # Panics
    ///
    /// Panics if this is `Continue` or `Break`.
    #[inline]
    pub fn unwrap_value(self) -> T {
        match self {
            __LoopSignal::Value(v) => v,
            __LoopSignal::Continue => panic!("called unwrap_value on Continue signal"),
            __LoopSignal::Break => panic!("called unwrap_value on Break signal"),
        }
    }
}

/// Helper to create Ok(LoopSignal::Continue) with inferred type.
/// Used by transformed control flow in signal mode handlers.
#[doc(hidden)]
#[inline]
pub fn __signal_continue<T>() -> core::result::Result<__LoopSignal<T>, Handled> {
    core::result::Result::Ok(__LoopSignal::Continue)
}

/// Non-generic control flow signal.
/// Used when the value type cannot be inferred (e.g., pure control flow handlers).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum __ControlSignal {
    /// Normal completion - value stored separately.
    Value,
    /// Signal to execute `continue` on the target loop.
    Continue,
    /// Signal to execute `break` on the target loop.
    Break,
}

/// Result type for control flow signals where value is stored separately.
#[doc(hidden)]
pub type __ControlResult = core::result::Result<__ControlSignal, Handled>;

/// Helper to return Continue signal.
#[doc(hidden)]
#[inline]
pub fn __ctrl_continue() -> __ControlResult {
    core::result::Result::Ok(__ControlSignal::Continue)
}

/// Helper to return Break signal.
#[doc(hidden)]
#[inline]
pub fn __ctrl_break() -> __ControlResult {
    core::result::Result::Ok(__ControlSignal::Break)
}

/// Helper to store a value and return Value signal.
/// Uses Option for better type inference than MaybeUninit.
#[doc(hidden)]
#[inline]
pub fn __ctrl_store_value<T>(slot: &mut Option<T>, value: T) -> __ControlResult {
    *slot = Some(value);
    core::result::Result::Ok(__ControlSignal::Value)
}

/// Creates an Option::None with type inferred from a Result reference.
/// Used to tie the Option's type to the body result's type.
#[doc(hidden)]
#[inline]
pub fn __ctrl_none_like<T, E>(_hint: &core::result::Result<T, E>) -> Option<T> {
    None
}

/// Identity function that forces type inference for Result.
/// Used to make type inference work for nested try blocks.
#[doc(hidden)]
#[inline]
pub fn __force_result_type<T, E>(result: core::result::Result<T, E>) -> core::result::Result<T, E> {
    result
}

/// Helper to create Ok(LoopSignal::Break) with inferred type.
/// Used by transformed control flow in signal mode handlers.
#[doc(hidden)]
#[inline]
pub fn __signal_break<T>() -> core::result::Result<__LoopSignal<T>, Handled> {
    core::result::Result::Ok(__LoopSignal::Break)
}

// ============================================================
// Try block macros
// ============================================================

/// Internal macro to create a try block that returns Result<T, Box<dyn Error>>.
/// Body is the success value - use `?` to propagate errors.
#[doc(hidden)]
#[macro_export]
macro_rules! __try_block {
    ($($body:tt)*) => {
        (|| -> ::core::result::Result<_, $crate::__BoxedError> {
            ::core::result::Result::Ok({ $($body)* })
        })()
    };
}

/// Internal macro for async try blocks.
#[doc(hidden)]
#[macro_export]
macro_rules! __async_try_block {
    ($($body:tt)*) => {
        (|| async move {
            let __result: ::core::result::Result<_, $crate::__BoxedError> =
                ::core::result::Result::Ok({ $($body)* });
            __result
        })()
    };
}

/// Async finally helper.
#[doc(hidden)]
#[inline]
pub async fn __with_finally_async<T, F, Fut, G>(f: F, finally: G) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
    G: FnOnce(),
{
    let result = f().await;
    finally();
    result
}
