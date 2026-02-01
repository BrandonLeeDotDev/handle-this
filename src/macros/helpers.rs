//! Helper functions and types for macros.

use crate::{Handled, Error};
use core::fmt;

/// Map error in a Result with boxed error, used by chain macros.
/// Uses map_err to eliminate identity Ok arm.
#[doc(hidden)]
#[cfg(feature = "std")]
#[inline]
pub fn __map_try_erased<T, F>(r: core::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>, on_err: F) -> core::result::Result<T, Handled<Error>>
where
    F: FnOnce(Handled<Error>) -> Handled<Error>,
{
    r.map_err(|e| on_err(Handled::wrap_box(e)))
}

/// Wrap a boxed error and add a frame in one call.
/// Uses optimized wrap_box_with_frame to avoid empty Vec + push pattern.
#[doc(hidden)]
#[cfg(feature = "std")]
#[inline]
pub fn __wrap_frame(e: Box<dyn std::error::Error + Send + Sync + 'static>, file: &'static str, line: u32, col: u32) -> Handled<Error> {
    Handled::wrap_box_with_frame(e, file, line, col)
}


// ============================================================
// Thrown - extracts Handled from Result or wraps errors
// Uses inherent impl + trait impl trick to handle both cases
// ============================================================

/// Wrapper for throw expressions. Uses inherent method priority
/// to handle both Result<T, E> and bare error types.
#[doc(hidden)]
pub struct __ThrowExpr<T>(pub T);

// Inherent impl for Result<T, Handled<E>> - takes priority over trait impl
impl<T, E: fmt::Display> __ThrowExpr<core::result::Result<T, Handled<E>>> {
    #[inline]
    pub fn __thrown(self) -> Handled<E> {
        match self.0 {
            Ok(_) => panic!("throw block returned Ok"),
            Err(e) => e,
        }
    }
}

// Inherent impl for Result<T, E> where E is a concrete error
impl<T, E: std::error::Error + Send + Sync + 'static> __ThrowExpr<core::result::Result<T, E>> {
    #[inline]
    pub fn __thrown_erased(self) -> Handled<Error> {
        match self.0 {
            Ok(_) => panic!("throw block returned Ok"),
            Err(e) => Handled::wrap(e),
        }
    }
}

// Inherent impl for type-erased Handled directly
impl __ThrowExpr<Handled<Error>> {
    #[inline]
    pub fn __thrown(self) -> Handled<Error> {
        self.0
    }
}

// Inherent impl for generic Handled<E>
impl<E: fmt::Display> __ThrowExpr<Handled<E>> {
    #[inline]
    pub fn __thrown_generic(self) -> Handled<E> {
        self.0
    }
}

// Inherent impl for &str - allows throw { "message" }
impl __ThrowExpr<&str> {
    #[inline]
    pub fn __thrown(self) -> Handled<Error> {
        Handled::msg(self.0)
    }
}

// Inherent impl for String - allows throw { format!(...) }
impl __ThrowExpr<String> {
    #[inline]
    pub fn __thrown(self) -> Handled<Error> {
        Handled::msg(self.0)
    }
}

/// Trait for converting error types to type-erased Handled.
/// Used as fallback when inherent impl doesn't match.
#[doc(hidden)]
pub trait __Thrown {
    fn __thrown(self) -> Handled<Error>;
}

// Trait impl for any Error type - used when not a Result
impl<E: std::error::Error + Send + Sync + 'static> __Thrown for __ThrowExpr<E> {
    #[inline]
    fn __thrown(self) -> Handled<Error> {
        Handled::wrap(self.0)
    }
}

// ============================================================
// Result helpers - reduce generated code for common patterns
// ============================================================

/// Run a computation and ensure a finally block runs regardless of result.
/// Replaces: `{ let __result = expr; let _ = { finally }; __result }`
#[doc(hidden)]
#[inline]
pub fn __with_finally<T, F, G>(f: F, finally: G) -> T
where
    F: FnOnce() -> T,
    G: FnOnce(),
{
    let result = f();
    finally();
    result
}

/// Convert a user's Result<T, E> to Result<T, Handled> for try catch blocks.
/// Uses Into<Handled> trait for error conversion.
#[doc(hidden)]
#[inline]
pub fn __convert_try_catch_result<T, E: Into<Handled<Error>>>(
    result: core::result::Result<T, E>,
    file: &'static str,
    line: u32,
    col: u32,
) -> core::result::Result<T, Handled<Error>> {
    result.map_err(|e| e.into().frame(file, line, col))
}

/// Convert a user's Result<T, &str> to Result<T, Handled> for try catch blocks.
/// Specialized version for string literal errors.
#[doc(hidden)]
#[inline]
pub fn __convert_try_catch_result_str<T>(
    result: core::result::Result<T, &str>,
    file: &'static str,
    line: u32,
    col: u32,
) -> core::result::Result<T, Handled<Error>> {
    result.map_err(|e| Handled::msg(e).frame(file, line, col))
}

/// Trait for converting try catch body results to Result<T, Handled>.
#[doc(hidden)]
pub trait TryCatchResult<T> {
    fn into_handled_result(self, file: &'static str, line: u32, col: u32) -> core::result::Result<T, Handled<Error>>;
}

// Blanket impl for any Result<T, E> where E: Into<Handled>
impl<T, E: Into<Handled<Error>>> TryCatchResult<T> for core::result::Result<T, E> {
    #[inline]
    fn into_handled_result(self, file: &'static str, line: u32, col: u32) -> core::result::Result<T, Handled<Error>> {
        self.map_err(|e| e.into().frame(file, line, col))
    }
}

/// Helper for try catch conversion. Provides default error type for inference.
#[doc(hidden)]
pub struct TryCatchConvert;

impl TryCatchConvert {
    /// Convert a try catch body. Error type is inferred from the body.
    #[inline]
    pub fn run<T, E: Into<Handled<Error>>>(
        result: core::result::Result<T, E>,
        file: &'static str,
        line: u32,
        col: u32,
    ) -> core::result::Result<T, Handled<Error>> {
        result.map_err(|e| e.into().frame(file, line, col))
    }
}

/// Wrapper that coerces ANY error type for try catch inference.
/// Uses Box<dyn Display> as the error type to accept any Display type.
#[doc(hidden)]
#[inline]
pub fn __try_catch_any<T, E: core::fmt::Display + Send + Sync + 'static, F>(
    f: F,
    file: &'static str,
    line: u32,
    col: u32,
) -> core::result::Result<T, Handled<Error>>
where
    F: FnOnce() -> core::result::Result<T, E>,
{
    f().map_err(|e| Handled::msg(e.to_string()).frame(file, line, col))
}

// ============================================================
// Error wrapper for method-based conversion
// Uses inherent impls (highest priority) for specific types,
// trait impl as fallback for any E: Error
// ============================================================

/// Wrapper for error conversion. Uses method resolution priority:
/// inherent impls for specific types, trait impl for generic errors.
#[doc(hidden)]
pub struct __ErrWrap<E>(pub E);

// Inherent impl for &str - highest priority
impl __ErrWrap<&str> {
    #[inline]
    pub fn __into_handled(self) -> Handled<Error> {
        Handled::msg(self.0)
    }
}

// Inherent impl for String
impl __ErrWrap<String> {
    #[inline]
    pub fn __into_handled(self) -> Handled<Error> {
        Handled::msg(self.0)
    }
}

// Inherent impl for Box<dyn Error>
impl __ErrWrap<Box<dyn std::error::Error + Send + Sync + 'static>> {
    #[inline]
    pub fn __into_handled(self) -> Handled<Error> {
        Handled::wrap_box(self.0)
    }
}

// Inherent impl for Handled - pass through
impl __ErrWrap<Handled<Error>> {
    #[inline]
    pub fn __into_handled(self) -> Handled<Error> {
        self.0
    }
}

/// Trait for fallback conversion of any Error type.
#[doc(hidden)]
pub trait __IntoHandled {
    fn __into_handled(self) -> Handled<Error>;
}

impl<E: std::error::Error + Send + Sync + 'static> __IntoHandled for __ErrWrap<E> {
    #[inline]
    fn __into_handled(self) -> Handled<Error> {
        Handled::wrap(self.0)
    }
}

