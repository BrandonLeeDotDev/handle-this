//! Unified action code generation.
//!
//! This module provides a single source of truth for generating handler action code.
//! Previously, the same patterns were duplicated across:
//! - `checks.rs` (closure mode)
//! - `error_handler.rs` (loop closure mode)
//! - `signal_handler.rs` (signal mode)
//!
//! ## Action Types
//!
//! - `ReturnOk` - Return Ok(body), used by catch handlers
//! - `Transform` - Transform error and continue chain, used by throw handlers
//! - `Execute` - Execute body without return, used by inspect handlers
//! - `ReturnDirect` - Return body directly (body is Result), used by try catch handlers
//!
//! ## Generation Modes
//!
//! - `Closure` - Standard closure mode with `return` statements
//! - `Signal` - Loop signal mode that wraps results in `LoopSignal`
//!
//! ## Note on Throw Semantics
//!
//! In Closure mode, throw uses `chain_after` to preserve the original error chain.
//! In Signal mode, throw currently does NOT use `chain_after` (historical behavior).
//! This may be a bug - see signal_handler.rs for the original implementation.

use proc_macro2::TokenStream;
use quote::quote;

/// What action to take when a handler check matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckAction {
    /// Return Ok(body) - used by catch
    ReturnOk,
    /// Transform error and continue chain - used by throw
    /// Reassigns __err to the new error, chain continues to next handler
    Transform,
    /// Execute body, don't return - used by inspect
    Execute,
    /// Return body directly (body is Result) - used by try catch
    ReturnDirect,
}

/// Code generation mode - determines how actions are wrapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMode {
    /// Standard closure mode - uses `return` statements, `chain_after` for throw
    Closure,
    /// Signal mode - wraps catch results in `LoopSignal::Value`
    Signal,
}

/// Configuration for action code generation.
#[derive(Debug, Clone)]
pub struct ActionConfig<'a> {
    /// The generation mode
    pub mode: ActionMode,
    /// Context chain for signal mode throw (e.g., `.with_context(...)`)
    /// Only used in Signal mode for Transform action
    pub ctx_chain: Option<&'a TokenStream>,
}

impl<'a> ActionConfig<'a> {
    /// Create a new closure mode config.
    pub fn closure() -> Self {
        Self {
            mode: ActionMode::Closure,
            ctx_chain: None,
        }
    }

    /// Create a new signal mode config.
    pub fn signal(ctx_chain: &'a TokenStream) -> Self {
        Self {
            mode: ActionMode::Signal,
            ctx_chain: Some(ctx_chain),
        }
    }
}

/// Generate action code for the given action type and configuration.
///
/// This is the core function that replaces the duplicated action code patterns
/// in checks.rs, error_handler.rs, and signal_handler.rs.
///
/// # Arguments
///
/// * `action` - The type of action to generate
/// * `body` - The body expression to execute
/// * `config` - Generation mode and context
///
/// # Returns
///
/// TokenStream containing the generated action code.
pub fn gen_action_code(
    action: CheckAction,
    body: &TokenStream,
    config: &ActionConfig,
) -> TokenStream {
    match config.mode {
        ActionMode::Closure => gen_closure_action(action, body),
        ActionMode::Signal => gen_signal_action(action, body, config.ctx_chain),
    }
}

/// Generate action code for match expressions.
///
/// Similar to `gen_action_code` but wraps the body in a match expression.
pub fn gen_match_action_code(
    action: CheckAction,
    expr: &TokenStream,
    arms: &TokenStream,
    config: &ActionConfig,
) -> TokenStream {
    match config.mode {
        ActionMode::Closure => gen_closure_match_action(action, expr, arms),
        ActionMode::Signal => gen_signal_match_action(action, expr, arms, config.ctx_chain),
    }
}

// ============================================================
// Closure Mode Implementation
// ============================================================

/// Generate closure mode action code.
fn gen_closure_action(action: CheckAction, body: &TokenStream) -> TokenStream {
    match action {
        CheckAction::ReturnOk => quote! {
            #[allow(unreachable_code)]
            return ::core::result::Result::Ok({ #body });
        },
        CheckAction::Transform => quote! {
            {
                #[allow(unused_imports)]
                use ::handle_this::__Thrown;
                let __new_err = ::handle_this::__ThrowExpr({ #body }).__thrown()
                    .frame(file!(), line!(), column!());
                __err = __new_err.chain_after(__err);
            }
        },
        CheckAction::Execute => quote! {
            { #body }
        },
        CheckAction::ReturnDirect => quote! {
            #[allow(unreachable_code)]
            return {
                #[allow(unused_imports)]
                use ::handle_this::result::{Ok, Err};
                #body
            };
        },
    }
}

/// Generate closure mode match action code.
fn gen_closure_match_action(
    action: CheckAction,
    expr: &TokenStream,
    arms: &TokenStream,
) -> TokenStream {
    match action {
        CheckAction::ReturnOk => quote! {
            #[allow(unreachable_code)]
            return ::core::result::Result::Ok(match #expr { #arms });
        },
        CheckAction::Transform => quote! {
            {
                #[allow(unused_imports)]
                use ::handle_this::__Thrown;
                let __new_err = ::handle_this::__ThrowExpr(match #expr { #arms }).__thrown()
                    .frame(file!(), line!(), column!());
                __err = __new_err.chain_after(__err);
            }
        },
        CheckAction::Execute => quote! {
            match #expr { #arms }
        },
        CheckAction::ReturnDirect => quote! {
            #[allow(unreachable_code)]
            return {
                #[allow(unused_imports)]
                use ::handle_this::result::{Ok, Err};
                match #expr { #arms }
            };
        },
    }
}

// ============================================================
// Signal Mode Implementation
// ============================================================

/// Generate signal mode action code.
///
/// Signal mode is used when handlers contain control flow (break/continue).
/// Catch results are wrapped in `LoopSignal::Value`.
fn gen_signal_action(
    action: CheckAction,
    body: &TokenStream,
    ctx_chain: Option<&TokenStream>,
) -> TokenStream {
    let signal = quote! { ::handle_this::__LoopSignal };
    let ctx = ctx_chain.map(|c| quote! { #c }).unwrap_or_default();

    match action {
        CheckAction::ReturnOk => quote! {
            let __handler_result = { #body };
            #[allow(unreachable_code)]
            return ::core::result::Result::Ok(#signal::Value(__handler_result));
        },
        // Note: Signal mode throw does NOT use chain_after (historical behavior)
        // This differs from closure mode - may need review
        CheckAction::Transform => {
            // Check if body contains signal helpers or LoopSignal:: (transformed control flow)
            // If so, just execute the body - it contains return statements that escape
            let body_str = body.to_string();
            if body_str.contains("LoopSignal") || body_str.contains("__LoopSignal")
                || body_str.contains("__signal_break") || body_str.contains("__signal_continue") {
                // Body was transformed from break/continue - just execute it
                quote! { { #body } }
            } else {
                // Normal throw - transform error
                quote! {
                    {
                        #[allow(unused_imports)]
                        use ::handle_this::__Thrown;
                        __err = ::handle_this::__ThrowExpr({ #body }).__thrown()
                            .frame(file!(), line!(), column!())
                            #ctx;
                    }
                }
            }
        },
        CheckAction::Execute => quote! {
            let _ = { #body };
        },
        CheckAction::ReturnDirect => quote! {
            #[allow(unreachable_code)]
            return {
                #[allow(unused_imports)]
                use ::handle_this::result::{Ok, Err};
                #body
            };
        },
    }
}

/// Generate signal mode match action code.
fn gen_signal_match_action(
    action: CheckAction,
    expr: &TokenStream,
    arms: &TokenStream,
    ctx_chain: Option<&TokenStream>,
) -> TokenStream {
    let signal = quote! { ::handle_this::__LoopSignal };
    let ctx = ctx_chain.map(|c| quote! { #c }).unwrap_or_default();

    match action {
        CheckAction::ReturnOk => quote! {
            #[allow(unreachable_code)]
            return ::core::result::Result::Ok(#signal::Value(match #expr { #arms }));
        },
        CheckAction::Transform => {
            // Check if arms contain signal helpers or LoopSignal:: (transformed control flow)
            let arms_str = arms.to_string();
            if arms_str.contains("LoopSignal") || arms_str.contains("__LoopSignal")
                || arms_str.contains("__signal_break") || arms_str.contains("__signal_continue") {
                // Match arms have control flow - just execute the match
                quote! { match #expr { #arms } }
            } else {
                quote! {
                    {
                        #[allow(unused_imports)]
                        use ::handle_this::__Thrown;
                        __err = ::handle_this::__ThrowExpr(match #expr { #arms }).__thrown()
                            .frame(file!(), line!(), column!())
                            #ctx;
                    }
                }
            }
        },
        CheckAction::Execute => quote! {
            match #expr { #arms }
        },
        CheckAction::ReturnDirect => quote! {
            #[allow(unreachable_code)]
            return {
                #[allow(unused_imports)]
                use ::handle_this::result::{Ok, Err};
                match #expr { #arms }
            };
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_closure_return_ok() {
        let body = quote! { 42 };
        let config = ActionConfig::closure();
        let code = gen_action_code(CheckAction::ReturnOk, &body, &config);
        let code_str = code.to_string();
        assert!(code_str.contains("return"));
        assert!(code_str.contains("Ok"));
    }

    #[test]
    fn test_signal_return_ok() {
        let body = quote! { 42 };
        let ctx = quote! {};
        let config = ActionConfig::signal(&ctx);
        let code = gen_action_code(CheckAction::ReturnOk, &body, &config);
        let code_str = code.to_string();
        assert!(code_str.contains("LoopSignal"));
        assert!(code_str.contains("Value"));
    }
}
