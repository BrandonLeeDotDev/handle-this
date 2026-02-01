//! Shared check generators for try patterns.
//!
//! Extracts the common pattern from sync_try's 12 `gen_*_check` functions.
//! Each check: type match → bind → guard check → action.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::codegen::action::{self, ActionConfig};
use crate::codegen::check::{wrap_with_type_check, gen_typed_inner, TypeCheckMode, gen_catchall_bindings, CatchallBindingConfig};
use crate::codegen::guard::{wrap_with_guard_separate_bindings, GuardContext};
use crate::keywords::{ChainVariant, Guard};
use crate::nested::transform_nested;

// Re-export CheckAction from codegen for backward compatibility
pub use crate::codegen::action::CheckAction;

/// Generate action code without guard (for use in loops where guard is checked separately).
fn gen_action_code(act: CheckAction, body: &TokenStream) -> TokenStream {
    action::gen_action_code(act, body, &ActionConfig::closure())
}

/// Generate match action code (for match guards in loops).
fn gen_match_action(act: CheckAction, expr: &TokenStream, arms: &TokenStream) -> TokenStream {
    action::gen_match_action_code(act, expr, arms, &ActionConfig::closure())
}

/// Generate a typed check (downcast_ref, chain_any, or chain_all).
pub fn gen_typed_check(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    act: CheckAction,
) -> TokenStream {
    let body = transform_nested(body.clone());
    let config = ActionConfig::closure();

    match variant {
        ChainVariant::Root => {
            let inner = gen_typed_inner(variant, binding, guard, &body, act, &config);
            wrap_with_type_check(variant, type_path, binding, &inner, TypeCheckMode::DowncastRoot)
        }
        ChainVariant::Any => {
            // For `any` variant with a guard, we need to check ALL matches in the chain
            // until we find one where the guard succeeds. Without a guard, first match wins.
            match guard {
                Some(Guard::When(condition)) => {
                    // Transform needs borrow-avoidance: compute while iterating, assign after
                    match act {
                        CheckAction::Transform => {
                            quote! {
                                {
                                    let mut __transform_result: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;
                                    for __typed_err in __err.chain_all::<#type_path>() {
                                        let #binding = __typed_err;
                                        if #condition {
                                            #[allow(unused_imports)]
                                            use ::handle_this::__Thrown;
                                            __transform_result = ::core::option::Option::Some(
                                                ::handle_this::__ThrowExpr({ #body }).__thrown()
                                                    .frame(file!(), line!(), column!())
                                            );
                                            break;
                                        }
                                    }
                                    if let ::core::option::Option::Some(__new_err) = __transform_result {
                                        __err = __new_err.chain_after(__err);
                                    }
                                }
                            }
                        }
                        _ => {
                            let action_code = gen_action_code(act, &body);
                            quote! {
                                for __typed_err in __err.chain_all::<#type_path>() {
                                    let #binding = __typed_err;
                                    if #condition {
                                        #action_code
                                    }
                                }
                            }
                        }
                    }
                }
                Some(Guard::Match { expr, arms }) => {
                    match act {
                        CheckAction::Transform => {
                            quote! {
                                {
                                    let mut __transform_result: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;
                                    for __typed_err in __err.chain_all::<#type_path>() {
                                        let #binding = __typed_err;
                                        #[allow(unused_imports)]
                                        use ::handle_this::__Thrown;
                                        __transform_result = ::core::option::Option::Some(
                                            ::handle_this::__ThrowExpr(match #expr { #arms }).__thrown()
                                                .frame(file!(), line!(), column!())
                                        );
                                        break;
                                    }
                                    if let ::core::option::Option::Some(__new_err) = __transform_result {
                                        __err = __new_err.chain_after(__err);
                                    }
                                }
                            }
                        }
                        _ => {
                            let match_action = gen_match_action(act, expr, arms);
                            quote! {
                                for __typed_err in __err.chain_all::<#type_path>() {
                                    let #binding = __typed_err;
                                    #match_action
                                }
                            }
                        }
                    }
                }
                None => {
                    // No guard - first match wins, use standard pattern
                    let inner = gen_typed_inner(variant, binding, guard, &body, act, &config);
                    wrap_with_type_check(variant, type_path, binding, &inner, TypeCheckMode::ChainRoot)
                }
            }
        }
        ChainVariant::All => {
            let inner = gen_typed_inner(variant, binding, guard, &body, act, &config);
            wrap_with_type_check(variant, type_path, binding, &inner, TypeCheckMode::ChainRoot)
        }
    }
}

/// Generate a catchall check (no type filter).
pub fn gen_catchall_check(
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    act: CheckAction,
    consume_binding: bool,
) -> TokenStream {
    let body = transform_nested(body.clone());
    let binding_str = binding.to_string();
    let config = ActionConfig::closure();

    // Special case: no guard and underscore binding - just return action directly
    if guard.is_none() && binding_str == "_" {
        return gen_action_code(act, &body);
    }

    let binding_config = if consume_binding {
        CatchallBindingConfig::catch()
    } else {
        CatchallBindingConfig::borrow()
    };
    let bindings = gen_catchall_bindings(binding, binding_config);

    wrap_with_guard_separate_bindings(
        guard,
        &GuardContext {
            action: act,
            body: &body,
            bind_stmt: &bindings.bind_stmt,
            action_config: &config,
        },
        &bindings.ref_bind,
        &bindings.action_bind,
    )
}
