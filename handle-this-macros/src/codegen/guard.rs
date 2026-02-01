//! Unified guard wrapping for handler code generation.
//!
//! This module provides a single source of truth for wrapping action code
//! with guard conditions (when/match). Previously this pattern was duplicated
//! across checks.rs, error_handler.rs, signal_handler.rs, and others.
//!
//! ## Guard Types
//!
//! - `When(condition)` - Wraps action in `if condition { action }`
//! - `Match { expr, arms }` - Uses match expression version of action
//! - `None` - Executes action directly
//!
//! ## Usage
//!
//! ```ignore
//! use crate::codegen::guard::{wrap_with_guard, GuardContext};
//!
//! let code = wrap_with_guard(
//!     &guard,
//!     &GuardContext {
//!         action: CheckAction::ReturnOk,
//!         body: &body,
//!         bind_stmt: &bind_stmt,
//!         action_config: &ActionConfig::closure(),
//!     },
//! );
//! ```

use proc_macro2::TokenStream;
use quote::quote;

use crate::keywords::Guard;
use super::action::{self, ActionConfig, CheckAction};

/// Context for guard wrapping.
pub struct GuardContext<'a> {
    /// The action type to execute
    pub action: CheckAction,
    /// The body expression for the action
    pub body: &'a TokenStream,
    /// Binding statement to include before the action
    pub bind_stmt: &'a TokenStream,
    /// Action configuration (closure vs signal mode)
    pub action_config: &'a ActionConfig<'a>,
}

/// Wrap action code with guard conditions.
///
/// This is the primary function for guard wrapping. It handles:
/// - `When` guards: wraps in `if condition { ... }`
/// - `Match` guards: uses match expression version of action
/// - No guard: just binding + action
///
/// Special handling for `Execute` action: inlines body directly to avoid nested blocks.
pub fn wrap_with_guard(
    guard: &Option<Guard>,
    ctx: &GuardContext,
) -> TokenStream {
    let action_code = action::gen_action_code(ctx.action, ctx.body, ctx.action_config);
    let bind_stmt = ctx.bind_stmt;
    let body = ctx.body;

    match guard {
        Some(Guard::When(condition)) => {
            // For Execute action, inline body to avoid nested blocks
            let guarded_action = match ctx.action {
                CheckAction::Execute => quote! {
                    if #condition {
                        #body
                    }
                },
                _ => quote! {
                    if #condition {
                        #action_code
                    }
                },
            };
            quote! {
                #bind_stmt
                #guarded_action
            }
        }
        Some(Guard::Match { expr, arms }) => {
            let match_action = action::gen_match_action_code(ctx.action, expr, arms, ctx.action_config);
            quote! {
                #bind_stmt
                #match_action
            }
        }
        None => {
            // For Execute action, inline body to avoid nested blocks
            match ctx.action {
                CheckAction::Execute => quote! {
                    #bind_stmt
                    #body
                },
                _ => quote! {
                    #bind_stmt
                    #action_code
                },
            }
        }
    }
}

/// Wrap action code with guard conditions, with custom binding for guard condition.
///
/// Used for catchall handlers where the guard condition needs a reference binding
/// but the action may need a different (owned/ref) binding.
///
/// # Arguments
/// - `guard` - The guard to apply
/// - `ctx` - The guard context
/// - `ref_bind` - Reference binding for guard condition evaluation
/// - `action_bind` - Binding to use when executing the action
pub fn wrap_with_guard_separate_bindings(
    guard: &Option<Guard>,
    ctx: &GuardContext,
    ref_bind: &TokenStream,
    action_bind: &TokenStream,
) -> TokenStream {
    let action_code = action::gen_action_code(ctx.action, ctx.body, ctx.action_config);
    let body = ctx.body;

    match guard {
        Some(Guard::When(condition)) => {
            let guarded_action = match ctx.action {
                CheckAction::Execute => quote! {
                    if #condition {
                        #action_bind
                        #body
                    }
                },
                _ => quote! {
                    if #condition {
                        #action_bind
                        #action_code
                    }
                },
            };
            quote! {
                {
                    #ref_bind
                    #guarded_action
                }
            }
        }
        Some(Guard::Match { expr, arms }) => {
            let match_action = action::gen_match_action_code(ctx.action, expr, arms, ctx.action_config);
            quote! {
                {
                    #action_bind
                    #match_action
                }
            }
        }
        None => {
            let bind_stmt = ctx.bind_stmt;
            // For Execute action, inline body to avoid nested blocks
            match ctx.action {
                CheckAction::Execute => quote! {
                    {
                        #bind_stmt
                        #body
                    }
                },
                _ => quote! {
                    {
                        #bind_stmt
                        #action_code
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_guard() {
        let body = quote! { 42 };
        let bind_stmt = quote! { let x = 1; };
        let config = ActionConfig::closure();
        let ctx = GuardContext {
            action: CheckAction::ReturnOk,
            body: &body,
            bind_stmt: &bind_stmt,
            action_config: &config,
        };

        let code = wrap_with_guard(&None, &ctx);
        let code_str = code.to_string();
        assert!(code_str.contains("let x = 1"));
        assert!(code_str.contains("return"));
    }

    #[test]
    fn test_when_guard() {
        let body = quote! { 42 };
        let bind_stmt = quote! {};
        let config = ActionConfig::closure();
        let condition = quote! { true };
        let guard = Some(Guard::When(condition));
        let ctx = GuardContext {
            action: CheckAction::ReturnOk,
            body: &body,
            bind_stmt: &bind_stmt,
            action_config: &config,
        };

        let code = wrap_with_guard(&guard, &ctx);
        let code_str = code.to_string();
        assert!(code_str.contains("if true"));
    }
}
