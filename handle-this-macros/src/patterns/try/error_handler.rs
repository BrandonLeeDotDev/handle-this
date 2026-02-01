//! Shared error handler generation for try patterns.
//!
//! Generates handler code for iteration patterns (try for, try while, try all).
//! Supports multiple handlers in declaration order.

use proc_macro2::TokenStream;
use quote::quote;

use crate::codegen::action::{self, ActionConfig, CheckAction};
use crate::codegen::check::{wrap_with_type_check, gen_typed_inner, TypeCheckMode, gen_catchall_bindings, CatchallBindingConfig};
use crate::keywords::{GenContext, ChainVariant};
use crate::keywords::with_ctx;
use crate::nested::transform_nested;
use super::handlers::{Handlers, Handler};

/// Generate error handler code for loop patterns (uses chain_any for type dispatch).
///
/// Loop patterns chain errors together, so Root variant should use chain_any
/// to search the chain rather than downcast_ref on the root.
pub fn generate_for_loop(handlers: &Handlers, ctx: &GenContext) -> TokenStream {
    let ctx_chain = with_ctx::gen_ctx_chain(ctx);

    // If no handlers, just propagate the error
    if handlers.handlers.is_empty() {
        return quote! {
            ::core::result::Result::Err(__err #ctx_chain)
        };
    }

    // Generate checks for each handler in declaration order
    let mut all_checks = Vec::new();

    for handler in &handlers.handlers {
        match handler {
            Handler::Catch(catch) => {
                let binding = &catch.binding;
                let body = transform_nested(catch.body.clone());

                let check = match (&catch.type_path, catch.variant) {
                    // Catch-all
                    (None, ChainVariant::Root) => {
                        gen_loop_catchall_check(binding, &catch.guard, &body, CheckAction::ReturnOk, true)
                    }
                    // Typed catch - use chain_any for Root variant in loops
                    (Some(type_path), variant) => {
                        gen_loop_typed_check(variant, type_path, binding, &catch.guard, &body, CheckAction::ReturnOk)
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
                let throw_expr = transform_nested(throw.throw_expr.clone());
                let binding = throw.binding.as_ref();

                let check = match (&throw.type_path, throw.variant) {
                    // Untyped throw - transforms error
                    (None, _) => {
                        let binding_ident = binding.cloned().unwrap_or_else(|| {
                            syn::Ident::new("_", proc_macro2::Span::call_site())
                        });
                        gen_loop_catchall_check(&binding_ident, &throw.guard, &throw_expr, CheckAction::Transform, false)
                    }
                    // Typed throw - use chain_any for Root variant in loops
                    (Some(type_path), variant) => {
                        let binding_ident = binding.cloned().unwrap_or_else(|| {
                            syn::Ident::new("_", proc_macro2::Span::call_site())
                        });
                        gen_loop_typed_check(variant, type_path, &binding_ident, &throw.guard, &throw_expr, CheckAction::Transform)
                    }
                };
                all_checks.push(check);
            }
            Handler::Inspect(inspect) => {
                let binding = &inspect.binding;
                let body = transform_nested(inspect.body.clone());

                let check = match (&inspect.type_path, inspect.variant) {
                    // Untyped inspect
                    (None, _) => {
                        gen_loop_catchall_check(binding, &inspect.guard, &body, CheckAction::Execute, false)
                    }
                    // Typed inspect - use chain_any for Root variant in loops
                    (Some(type_path), variant) => {
                        gen_loop_typed_check(variant, type_path, binding, &inspect.guard, &body, CheckAction::Execute)
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
        ::core::result::Result::Err(__err #ctx_chain)
    };

    quote! {
        #(#all_checks)*
        #fallback
    }
}

// ============================================================
// Loop-specific check generators (use chain_any for Root variant)
// ============================================================

use crate::codegen::guard::{wrap_with_guard_separate_bindings, GuardContext};
use crate::keywords::Guard;

/// Generate action code for loop handlers (closure mode).
fn gen_action_code(act: CheckAction, body: &TokenStream) -> TokenStream {
    action::gen_action_code(act, body, &ActionConfig::closure())
}

/// Generate a typed check for loop patterns.
/// Uses chain_any for Root variant (since loop errors are chained).
fn gen_loop_typed_check(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    act: CheckAction,
) -> TokenStream {
    let config = ActionConfig::closure();

    // Transform action needs special borrow-avoidance handling
    if act == CheckAction::Transform && matches!(variant, ChainVariant::Root | ChainVariant::Any) {
        let transform_body = gen_loop_transform_body(guard, binding, body);
        return quote! {
            {
                let __transform_result: ::core::option::Option<::handle_this::Handled> =
                    if let ::core::option::Option::Some(__typed_err) = __err.chain_any::<#type_path>() {
                        #transform_body
                    } else {
                        ::core::option::Option::None
                    };
                if let ::core::option::Option::Some(__new_err) = __transform_result {
                    __err = __new_err.chain_after(__err);
                }
            }
        };
    }

    // Non-Transform cases use standard type check pattern
    let inner = gen_typed_inner(variant, binding, guard, body, act, &config);
    wrap_with_type_check(variant, type_path, binding, &inner, TypeCheckMode::ChainRoot)
}

/// Generate the transform body for loop typed checks.
/// Returns Some(new_error) if transform should happen, None otherwise.
fn gen_loop_transform_body(
    guard: &Option<Guard>,
    binding: &syn::Ident,
    body: &TokenStream,
) -> TokenStream {
    let bind_stmt = quote! { let #binding = __typed_err; };
    let create_error = quote! {
        {
            #[allow(unused_imports)]
            use ::handle_this::__Thrown;
            ::core::option::Option::Some(
                ::handle_this::__ThrowExpr({ #body }).__thrown()
                    .frame(file!(), line!(), column!())
            )
        }
    };

    match guard {
        Some(Guard::When(condition)) => {
            quote! {
                #bind_stmt
                if #condition {
                    #create_error
                } else {
                    ::core::option::Option::None
                }
            }
        }
        Some(Guard::Match { expr, arms }) => {
            quote! {
                #bind_stmt
                ::core::option::Option::Some({
                    #[allow(unused_imports)]
                    use ::handle_this::__Thrown;
                    ::handle_this::__ThrowExpr(match #expr { #arms }).__thrown()
                        .frame(file!(), line!(), column!())
                })
            }
        }
        None => {
            quote! {
                #bind_stmt
                #create_error
            }
        }
    }
}

/// Generate a catchall check for loop patterns.
fn gen_loop_catchall_check(
    binding: &syn::Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    act: CheckAction,
    consume_binding: bool,
) -> TokenStream {
    let binding_str = binding.to_string();
    let config = ActionConfig::closure();

    // Special case: no guard and underscore binding - just return action directly
    if guard.is_none() && binding_str == "_" {
        return gen_action_code(act, body);
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
            body,
            bind_stmt: &bindings.bind_stmt,
            action_config: &config,
        },
        &bindings.ref_bind,
        &bindings.action_bind,
    )
}
