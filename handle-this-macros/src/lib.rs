//! Proc macros for handle-this crate.
//!
//! This crate provides a single entry point that routes to pattern-specific
//! handlers, which in turn use shared keyword modules.

use proc_macro::TokenStream;

mod router;
mod keywords;
mod patterns;
mod nested;
mod codegen;

/// Single proc macro entry point for all handle! patterns.
///
/// The declarative macro converts pattern keywords to markers:
/// - `try { }` -> `SYNC { }`
/// - `async try { }` -> `ASYNC { }`
/// - `try for` -> `FOR`
/// - `try any` -> `ANY`
/// - `try all` -> `ALL`
/// - `try while` -> `WHILE`
#[proc_macro]
pub fn __handle_proc(input: TokenStream) -> TokenStream {
    router::route(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

// ============================================================
// Pattern-specific entry points for nested patterns
// These are called directly by nested pattern transformation
// to avoid type inference issues with the main handle! router.
// ============================================================

/// Direct entry point for try for patterns.
#[proc_macro]
pub fn __try_for_proc(input: TokenStream) -> TokenStream {
    patterns::r#try::iter::process_for(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Direct entry point for try any patterns (alias for try for with first-success semantics).
#[proc_macro]
pub fn __try_any_proc(input: TokenStream) -> TokenStream {
    patterns::r#try::iter::process_any(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Direct entry point for try all patterns.
#[proc_macro]
pub fn __try_all_proc(input: TokenStream) -> TokenStream {
    patterns::r#try::iter::process_all(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Direct entry point for try while patterns.
#[proc_macro]
pub fn __try_while_proc(input: TokenStream) -> TokenStream {
    patterns::r#try::retry::process(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Direct entry point for sync try patterns (try { } catch/throw/...).
#[proc_macro]
pub fn __sync_try_proc(input: TokenStream) -> TokenStream {
    patterns::r#try::sync::process(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Direct entry point for async try patterns (async try { } catch/throw/...).
#[proc_macro]
pub fn __async_try_proc(input: TokenStream) -> TokenStream {
    patterns::r#try::async_impl::process(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

// ============================================================
// Then chain routing helpers
// These detect whether `then` appears and route to then_chain or regular pattern
// ============================================================

/// Route basic try with `with` clause - detect if `then` follows.
/// Input: BASIC { body } with ... [, then ...] or [handlers]
#[proc_macro]
pub fn __then_or_sync(input: TokenStream) -> TokenStream {
    router::route_then_or_sync(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Route iteration patterns - detect if `then` follows after body.
/// Input: FOR/ANY/ALL/WHILE ... { body } [, then ...] or [handlers]
#[proc_macro]
pub fn __then_or_iter(input: TokenStream) -> TokenStream {
    router::route_then_or_iter(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

