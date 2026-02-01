//! Shared signal-mode handler generation for loop patterns.
//!
//! Used by both `iter.rs` (try for/any/all) and `retry.rs` (try while).
//! Signal mode transforms control flow (continue/break) to signals,
//! allowing typed catches to propagate unmatched errors.
//!
//! Supports multiple handlers in declaration order.

use proc_macro2::TokenStream;
use quote::quote;

use crate::codegen::action::{ActionConfig, CheckAction};
use crate::codegen::check::{wrap_with_type_check, gen_typed_inner, TypeCheckMode, gen_catchall_bindings, CatchallBindingConfig};
use crate::codegen::guard::{wrap_with_guard_separate_bindings, GuardContext};
use crate::keywords::{ChainVariant, Guard};
use crate::nested::transform_nested;
use super::handlers::{Handlers, Handler};
use super::signal::{transform_control_flow, transform_control_flow_nongeneric, signal_type};

/// Generate the signal-mode handler chain.
///
/// Processes handlers in declaration order:
/// 1. Each handler is checked in sequence
/// 2. On match: catch returns `Ok(LoopSignal::Value(v))`, throw returns `Err(transformed)`
/// 3. On no match: falls through to next handler
/// 4. Final fallback: returns `Err(__err)` for unmatched errors
pub fn gen_signal_handler(
    handlers: &Handlers,
    ctx_chain: &TokenStream,
) -> TokenStream {
    let signal = signal_type();

    // If no handlers, just propagate the error
    if handlers.handlers.is_empty() {
        return quote! {
            return ::core::result::Result::Err(__err #ctx_chain);
        };
    }

    // Generate checks for each handler in declaration order
    let mut all_checks = Vec::new();

    for handler in &handlers.handlers {
        match handler {
            Handler::Catch(catch) => {
                let binding = &catch.binding;
                let body = transform_control_flow(transform_nested(catch.body.clone()));

                let check = match (&catch.type_path, catch.variant) {
                    // Catch-all - always matches
                    (None, ChainVariant::Root) => {
                        gen_signal_catchall_catch(binding, &catch.guard, &body, &signal)
                    }
                    // Typed catch - may not match
                    (Some(type_path), variant) => {
                        gen_signal_typed_catch(variant, type_path, binding, &catch.guard, &body, &signal)
                    }
                    // Invalid: catch-all with any/all variant
                    (None, _) => {
                        syn::Error::new(binding.span(), "catch any/all requires a type")
                            .to_compile_error()
                    }
                };
                all_checks.push(check);
            }
            Handler::Throw(throw) => {
                let throw_expr = transform_control_flow(transform_nested(throw.throw_expr.clone()));
                let binding = throw.binding.as_ref();

                let check = match (&throw.type_path, throw.variant) {
                    // Untyped throw - always matches
                    (None, _) => {
                        let binding_ident = binding.cloned().unwrap_or_else(|| {
                            syn::Ident::new("_", proc_macro2::Span::call_site())
                        });
                        gen_signal_catchall_throw(&binding_ident, &throw.guard, &throw_expr, ctx_chain)
                    }
                    // Typed throw - may not match
                    (Some(type_path), variant) => {
                        let binding_ident = binding.cloned().unwrap_or_else(|| {
                            syn::Ident::new("_", proc_macro2::Span::call_site())
                        });
                        gen_signal_typed_throw(variant, type_path, &binding_ident, &throw.guard, &throw_expr, ctx_chain)
                    }
                };
                all_checks.push(check);
            }
            Handler::Inspect(inspect) => {
                let binding = &inspect.binding;
                let body = transform_control_flow(transform_nested(inspect.body.clone()));

                let check = match (&inspect.type_path, inspect.variant) {
                    // Untyped inspect
                    (None, _) => {
                        gen_signal_catchall_inspect(binding, &inspect.guard, &body)
                    }
                    // Typed inspect
                    (Some(type_path), variant) => {
                        gen_signal_typed_inspect(variant, type_path, binding, &inspect.guard, &body)
                    }
                };
                all_checks.push(check);
            }
            // TryCatch is not supported in loop patterns - parsing rejects it
            Handler::TryCatch(_) => unreachable!("try catch not supported in loop patterns"),
        }
    }

    // Always include fallback for proper type-checking, even when there's a catch-all.
    // The catch-all's return makes it unreachable, but the compiler still needs it.
    let fallback = quote! {
        return ::core::result::Result::Err(__err #ctx_chain);
    };

    quote! {
        #(#all_checks)*
        #fallback
    }
}

// ============================================================
// Catch handlers for signal mode
// ============================================================

/// Generate a catch-all catch handler for signal mode.
fn gen_signal_catchall_catch(
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    _signal: &TokenStream,
) -> TokenStream {
    let empty_ctx = quote! {};
    let config = ActionConfig::signal(&empty_ctx);
    let bindings = gen_catchall_bindings(binding, CatchallBindingConfig::catch());

    wrap_with_guard_separate_bindings(
        guard,
        &GuardContext {
            action: CheckAction::ReturnOk,
            body,
            bind_stmt: &bindings.bind_stmt,
            action_config: &config,
        },
        &bindings.ref_bind,
        &bindings.bind_stmt,  // catch uses owned binding for action too
    )
}

/// Generate a typed catch handler for signal mode.
fn gen_signal_typed_catch(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    _signal: &TokenStream,
) -> TokenStream {
    let empty_ctx = quote! {};
    let config = ActionConfig::signal(&empty_ctx);
    let inner = gen_typed_inner(variant, binding, guard, body, CheckAction::ReturnOk, &config);
    wrap_with_type_check(variant, type_path, binding, &inner, TypeCheckMode::ChainRoot)
}

// ============================================================
// Throw handlers for signal mode
// ============================================================

/// Generate a catch-all throw handler for signal mode.
/// Throw transforms the error and continues the chain (does not return).
fn gen_signal_catchall_throw(
    binding: &syn::Ident,
    guard: &Option<Guard>,
    throw_expr: &TokenStream,
    ctx_chain: &TokenStream,
) -> TokenStream {
    let config = ActionConfig::signal(ctx_chain);
    let bindings = gen_catchall_bindings(binding, CatchallBindingConfig::borrow());

    wrap_with_guard_separate_bindings(
        guard,
        &GuardContext {
            action: CheckAction::Transform,
            body: throw_expr,
            bind_stmt: &bindings.bind_stmt,
            action_config: &config,
        },
        &bindings.ref_bind,
        &quote! {},  // throw doesn't need additional binding in action
    )
}

/// Generate a typed throw handler for signal mode.
fn gen_signal_typed_throw(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &syn::Ident,
    guard: &Option<Guard>,
    throw_expr: &TokenStream,
    ctx_chain: &TokenStream,
) -> TokenStream {
    let config = ActionConfig::signal(ctx_chain);
    let inner = gen_typed_inner(variant, binding, guard, throw_expr, CheckAction::Transform, &config);
    wrap_with_type_check(variant, type_path, binding, &inner, TypeCheckMode::ChainRoot)
}

// ============================================================
// Inspect handlers for signal mode
// ============================================================

/// Generate a catch-all inspect handler for signal mode.
fn gen_signal_catchall_inspect(
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
) -> TokenStream {
    let empty_ctx = quote! {};
    let config = ActionConfig::signal(&empty_ctx);
    let bindings = gen_catchall_bindings(binding, CatchallBindingConfig::borrow());

    wrap_with_guard_separate_bindings(
        guard,
        &GuardContext {
            action: CheckAction::Execute,
            body,
            bind_stmt: &bindings.bind_stmt,
            action_config: &config,
        },
        &bindings.ref_bind,
        &quote! {},  // inspect doesn't need additional binding in action
    )
}

/// Generate a typed inspect handler for signal mode.
fn gen_signal_typed_inspect(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
) -> TokenStream {
    let empty_ctx = quote! {};
    let config = ActionConfig::signal(&empty_ctx);
    let inner = gen_typed_inner(variant, binding, guard, body, CheckAction::Execute, &config);
    wrap_with_type_check(variant, type_path, binding, &inner, TypeCheckMode::ChainRoot)
}

// ============================================================
// Control Signal Mode (non-generic)
// ============================================================

/// Generate control signal handler chain for pure control flow handlers.
/// Uses __ControlSignal instead of __LoopSignal<T> to avoid type inference issues.
pub fn gen_control_signal_handler(
    handlers: &Handlers,
    ctx_chain: &TokenStream,
) -> TokenStream {
    // If no handlers, just propagate the error
    if handlers.handlers.is_empty() {
        return quote! {
            return ::core::result::Result::Err(__err #ctx_chain);
        };
    }

    // Transform handler bodies: first nested patterns, then control flow to __ctrl_*
    let mut all_checks = Vec::new();

    for handler in &handlers.handlers {
        match handler {
            Handler::Catch(catch) => {
                let binding = &catch.binding;
                // Transform nested patterns, then convert break/continue to __ctrl_* calls
                let body = transform_control_flow_nongeneric(transform_nested(catch.body.clone()));

                let check = match (&catch.type_path, catch.variant) {
                    (None, ChainVariant::Root) => {
                        gen_control_catchall(binding, &catch.guard, &body)
                    }
                    (Some(type_path), variant) => {
                        gen_control_typed(variant, type_path, binding, &catch.guard, &body)
                    }
                    (None, _) => {
                        syn::Error::new(binding.span(), "catch any/all requires a type")
                            .to_compile_error()
                    }
                };
                all_checks.push(check);
            }
            Handler::Throw(throw) => {
                // Transform nested patterns, then convert break/continue to __ctrl_* calls
                let throw_expr = transform_control_flow_nongeneric(transform_nested(throw.throw_expr.clone()));
                let binding = throw.binding.as_ref();

                let check = match (&throw.type_path, throw.variant) {
                    (None, _) => {
                        let binding_ident = binding.cloned().unwrap_or_else(|| {
                            syn::Ident::new("_", proc_macro2::Span::call_site())
                        });
                        gen_control_catchall(&binding_ident, &throw.guard, &throw_expr)
                    }
                    (Some(type_path), variant) => {
                        let binding_ident = binding.cloned().unwrap_or_else(|| {
                            syn::Ident::new("_", proc_macro2::Span::call_site())
                        });
                        gen_control_typed(variant, type_path, &binding_ident, &throw.guard, &throw_expr)
                    }
                };
                all_checks.push(check);
            }
            Handler::Inspect(inspect) => {
                let binding = &inspect.binding;
                // Transform nested patterns, then convert break/continue to __ctrl_* calls
                let body = transform_control_flow_nongeneric(transform_nested(inspect.body.clone()));

                let check = match (&inspect.type_path, inspect.variant) {
                    (None, _) => {
                        gen_control_inspect(&binding, &inspect.guard, &body)
                    }
                    (Some(type_path), variant) => {
                        gen_control_typed_inspect(variant, type_path, binding, &inspect.guard, &body)
                    }
                };
                all_checks.push(check);
            }
            Handler::TryCatch(_) => unreachable!("try catch not supported in control signal mode"),
        }
    }

    // Fallback for unmatched errors
    let fallback = quote! {
        return ::core::result::Result::Err(__err #ctx_chain);
    };

    quote! {
        #(#all_checks)*
        #fallback
    }
}

/// Generate catch-all handler for control signal mode.
fn gen_control_catchall(
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
) -> TokenStream {
    let bind_stmt = if binding.to_string() == "_" {
        quote! {}
    } else {
        quote! { let #binding = __err; }
    };

    match guard {
        Some(Guard::When(cond)) => {
            let ref_bind = if binding.to_string() == "_" {
                quote! {}
            } else {
                quote! { let #binding = &__err; }
            };
            quote! {
                {
                    #ref_bind
                    if #cond {
                        #bind_stmt
                        { #body }
                    }
                }
            }
        }
        Some(Guard::Match { expr, arms }) => quote! {
            {
                #bind_stmt
                return match #expr { #arms };
            }
        },
        None => quote! {
            {
                #bind_stmt
                { #body }
            }
        },
    }
}

/// Generate typed handler for control signal mode.
fn gen_control_typed(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
) -> TokenStream {
    let type_check = match variant {
        ChainVariant::Root => quote! { __err.downcast_ref::<#type_path>() },
        ChainVariant::Any => quote! { __err.chain_any::<#type_path>() },
        ChainVariant::All => quote! { __err.chain_all::<#type_path>() },
    };

    let is_all = matches!(variant, ChainVariant::All);

    if is_all {
        match guard {
            Some(Guard::When(cond)) => quote! {
                {
                    let #binding: ::std::vec::Vec<&#type_path> = #type_check;
                    if !#binding.is_empty() && #cond {
                        { #body }
                    }
                }
            },
            Some(Guard::Match { expr, arms }) => quote! {
                {
                    let #binding: ::std::vec::Vec<&#type_path> = #type_check;
                    if !#binding.is_empty() {
                        return match #expr { #arms };
                    }
                }
            },
            None => quote! {
                {
                    let #binding: ::std::vec::Vec<&#type_path> = #type_check;
                    if !#binding.is_empty() {
                        { #body }
                    }
                }
            },
        }
    } else {
        match guard {
            Some(Guard::When(cond)) => quote! {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    if #cond {
                        { #body }
                    }
                }
            },
            Some(Guard::Match { expr, arms }) => quote! {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    return match #expr { #arms };
                }
            },
            None => quote! {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    { #body }
                }
            },
        }
    }
}

/// Generate inspect handler for control signal mode.
fn gen_control_inspect(
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
) -> TokenStream {
    let bind_stmt = if binding.to_string() == "_" {
        quote! {}
    } else {
        quote! { let #binding = &__err; }
    };

    match guard {
        Some(Guard::When(cond)) => quote! {
            {
                #bind_stmt
                if #cond {
                    { #body }
                }
            }
        },
        _ => quote! {
            {
                #bind_stmt
                { #body }
            }
        },
    }
}

/// Generate typed inspect handler for control signal mode.
fn gen_control_typed_inspect(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
) -> TokenStream {
    let type_check = match variant {
        ChainVariant::Root => quote! { __err.downcast_ref::<#type_path>() },
        ChainVariant::Any => quote! { __err.chain_any::<#type_path>() },
        ChainVariant::All => quote! { __err.chain_all::<#type_path>() },
    };

    let is_all = matches!(variant, ChainVariant::All);

    if is_all {
        match guard {
            Some(Guard::When(cond)) => quote! {
                {
                    let #binding: ::std::vec::Vec<&#type_path> = #type_check;
                    if !#binding.is_empty() && #cond {
                        { #body }
                    }
                }
            },
            _ => quote! {
                {
                    let #binding: ::std::vec::Vec<&#type_path> = #type_check;
                    if !#binding.is_empty() {
                        { #body }
                    }
                }
            },
        }
    } else {
        match guard {
            Some(Guard::When(cond)) => quote! {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    if #cond {
                        { #body }
                    }
                }
            },
            _ => quote! {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    { #body }
                }
            },
        }
    }
}
