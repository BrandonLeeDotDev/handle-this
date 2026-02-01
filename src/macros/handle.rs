//! The `handle!` macro - main entry point for error handling.
//!
//! This declarative macro catches pattern keywords (which proc macros can't parse)
//! and routes to the unified proc macro with pattern markers.

/// Main error handling macro.
///
/// # Patterns
///
/// ## Basic try
/// ```
/// use handle_this::{handle, Result};
///
/// fn example() -> Result<i32> {
///     handle! {
///         try { Ok::<_, &str>(42)? }
///         catch { -1 }
///     }
/// }
/// assert_eq!(example().unwrap(), 42);
/// ```
///
/// ## Try for (first success)
/// ```
/// use handle_this::{handle, Result};
///
/// fn example() -> Result<i32> {
///     let items: Vec<std::result::Result<i32, &str>> = vec![Err("a"), Ok(42)];
///     handle! {
///         try for item in items { item? }
///         catch { -1 }
///     }
/// }
/// assert_eq!(example().unwrap(), 42);
/// ```
///
/// ## Try all (collect all successes)
/// ```
/// use handle_this::{handle, Result};
///
/// fn example() -> Result<Vec<i32>> {
///     let items = vec![1, 2, 3];
///     handle! {
///         try all item in items { Ok::<_, &str>(item * 2)? }
///     }
/// }
/// assert_eq!(example().unwrap(), vec![2, 4, 6]);
/// ```
///
/// ## Try while (retry until success or condition false)
/// ```
/// use handle_this::{handle, Result};
///
/// fn example() -> Result<&'static str> {
///     let mut attempts = 0;
///     handle! {
///         try while attempts < 3 {
///             attempts += 1;
///             if attempts < 3 { Err("not yet")? }
///             "success"
///         }
///         catch { "gave up" }
///     }
/// }
/// assert_eq!(example().unwrap(), "success");
/// ```
#[macro_export]
macro_rules! handle {
    // ========================================
    // Preconditions (require)
    // ========================================

    // require COND else "msg", rest...
    (require $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(REQUIRE $($rest)+)
    };

    // scope "name", rest...
    (scope $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(SCOPE $($rest)+)
    };

    // ========================================
    // Conditional patterns
    // ========================================

    // try when CONDITION { } else when ... else { } handlers...
    (try when $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(WHEN $($rest)+)
    };

    // ========================================
    // Then chains, iteration patterns, and async
    // More specific patterns (with `, then`) must come first
    // ========================================

    // async try { } , then ... (must come before general async)
    (async try { $($body:tt)* } , then $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(THEN ASYNC { $($body)* } , then $($rest)+)
    };

    // async try { } handlers...
    (async try { $($body:tt)* } $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(ASYNC { $($body)* } $($rest)+)
    };

    // async try { } alone
    (async try { $($body:tt)* }) => {
        ::handle_this_macros::__handle_proc!(ASYNC { $($body)* })
    };

    // try { } , then ... (basic - must come before general try)
    (try { $($body:tt)* } , then $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(THEN BASIC { $($body)* } , then $($rest)+)
    };

    // try { } with ... (may or may not have then)
    (try { $($body:tt)* } with $($with_and_rest:tt)+) => {
        ::handle_this_macros::__then_or_sync!(BASIC { $($body)* } with $($with_and_rest)+)
    };

    // try -> T { } , then ... (direct mode with then)
    (try -> $type:ty { $($body:tt)* } , then $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(THEN DIRECT -> $type { $($body)* } , then $($rest)+)
    };

    // try for/any/all/while - route through proc macro for then detection
    (try for $($all:tt)+) => {
        ::handle_this_macros::__then_or_iter!(FOR $($all)+)
    };
    (try any $($all:tt)+) => {
        ::handle_this_macros::__then_or_iter!(ANY $($all)+)
    };
    (try all $($all:tt)+) => {
        ::handle_this_macros::__then_or_iter!(ALL $($all)+)
    };
    (try while $($all:tt)+) => {
        ::handle_this_macros::__then_or_iter!(WHILE $($all)+)
    };

    // ========================================
    // Basic sync pattern
    // ========================================

    // try -> Type { } handlers... (explicit direct mode)
    (try -> $type:ty { $($body:tt)* } $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(SYNC -> $type { $($body)* } $($rest)+)
    };

    // try -> Type { } alone (explicit direct mode, no handlers)
    (try -> $type:ty { $($body:tt)* }) => {
        ::handle_this_macros::__handle_proc!(SYNC -> $type { $($body)* })
    };

    // try { } handlers...
    (try { $($body:tt)* } $($rest:tt)+) => {
        ::handle_this_macros::__handle_proc!(SYNC { $($body)* } $($rest)+)
    };

    // try { } alone - just wraps with stack frame
    (try { $($body:tt)* }) => {
        ::handle_this_macros::__handle_proc!(SYNC { $($body)* })
    };

    // ========================================
    // Error catch-all - route to proc macro for better spans
    // ========================================

    // Catch-all: first token preserved with span, proc macro determines error type
    ($first:tt $($rest:tt)*) => {
        ::handle_this_macros::__handle_proc!(ERROR $first $($rest)*)
    };

    // Empty input
    () => {
        ::handle_this_macros::__handle_proc!(ERROR_EMPTY)
    };
}
