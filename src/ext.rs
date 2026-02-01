//! Extension trait for Result types.

use crate::handled::IntoValue;
use crate::Result;

/// Extension trait for adding context to `Result<T, Handled>`.
pub trait HandleExt<T> {
    /// Chain another operation, adding a frame on error.
    fn then<U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> Result<U>;

    /// Chain another operation with context message.
    fn then_with<U, F>(self, ctx: impl Into<String>, f: F) -> Result<U>
    where
        F: FnOnce(T) -> Result<U>;

    /// Add context message to error.
    fn context(self, ctx: impl Into<String>) -> Self;

    /// Add key-value attachment to error with typed value.
    fn attach(self, key: &'static str, val: impl IntoValue) -> Self;
}

impl<T> HandleExt<T> for Result<T> {
    #[track_caller]
    fn then<U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> Result<U>,
    {
        let loc = core::panic::Location::caller();
        match self {
            Ok(v) => f(v).map_err(|e| e.frame(loc.file(), loc.line(), loc.column())),
            Err(e) => Err(e),
        }
    }

    #[track_caller]
    fn then_with<U, F>(self, ctx: impl Into<String>, f: F) -> Result<U>
    where
        F: FnOnce(T) -> Result<U>,
    {
        let loc = core::panic::Location::caller();
        let ctx = ctx.into();
        match self {
            Ok(v) => f(v).map_err(|e| e.frame(loc.file(), loc.line(), loc.column()).ctx(ctx)),
            Err(e) => Err(e),
        }
    }

    #[track_caller]
    fn context(self, ctx: impl Into<String>) -> Self {
        let loc = core::panic::Location::caller();
        self.map_err(|e| e.frame(loc.file(), loc.line(), loc.column()).ctx(ctx))
    }

    #[track_caller]
    fn attach(self, key: &'static str, val: impl IntoValue) -> Self {
        let loc = core::panic::Location::caller();
        self.map_err(|e| e.frame(loc.file(), loc.line(), loc.column()).kv(key, val))
    }
}
