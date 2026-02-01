//! Unified type checking and binding generation.
//!
//! This module provides shared code for:
//! - Type dispatch wrappers (downcast_ref, chain_any, chain_all)
//! - Typed handler inner code generation
//! - Catchall binding generation
//!
//! ## Type Check Modes
//!
//! - `DowncastRoot` - Uses `downcast_ref` for Root variant (sync try pattern)
//! - `ChainRoot` - Uses `chain_any` for Root variant (loop patterns where errors are chained)

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::keywords::{ChainVariant, Guard};
use super::action::{ActionConfig, CheckAction};
use super::guard::{wrap_with_guard, GuardContext};

/// Mode for Root variant type checking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeCheckMode {
    /// Use `downcast_ref` for Root variant (sync try - single error)
    DowncastRoot,
    /// Use `chain_any` for Root variant (loop patterns - chained errors)
    ChainRoot,
}

/// Wrap inner code with type dispatch based on ChainVariant.
///
/// Generates the outer type-checking structure:
/// - Root: `if let Some(__typed_err) = __err.downcast_ref::<T>()` or `chain_any::<T>()`
/// - Any: `if let Some(__typed_err) = __err.chain_any::<T>()`
/// - All: `let binding: Vec<&T> = __err.chain_all::<T>(); if !binding.is_empty()`
///
/// The `inner` code is placed inside the conditional.
pub fn wrap_with_type_check(
    variant: ChainVariant,
    type_path: &TokenStream,
    binding: &Ident,
    inner: &TokenStream,
    mode: TypeCheckMode,
) -> TokenStream {
    match variant {
        ChainVariant::Root => {
            match mode {
                TypeCheckMode::DowncastRoot => {
                    quote! {
                        if let ::core::option::Option::Some(__typed_err) = __err.downcast_ref::<#type_path>() {
                            #inner
                        }
                    }
                }
                TypeCheckMode::ChainRoot => {
                    quote! {
                        if let ::core::option::Option::Some(__typed_err) = __err.chain_any::<#type_path>() {
                            #inner
                        }
                    }
                }
            }
        }
        ChainVariant::Any => {
            quote! {
                if let ::core::option::Option::Some(__typed_err) = __err.chain_any::<#type_path>() {
                    #inner
                }
            }
        }
        ChainVariant::All => {
            quote! {
                {
                    let #binding: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                    if !#binding.is_empty() {
                        #inner
                    }
                }
            }
        }
    }
}

/// Generate the inner handler code for typed checks.
///
/// Creates bind_stmt based on variant and wraps with guard.
/// For All variant, binding is already set by the outer wrapper.
pub fn gen_typed_inner(
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    action: CheckAction,
    config: &ActionConfig,
) -> TokenStream {
    let needs_binding = variant != ChainVariant::All;
    let bind_stmt = if needs_binding {
        quote! { let #binding = __typed_err; }
    } else {
        quote! {} // All variant already has binding set by wrap_with_type_check
    };

    wrap_with_guard(guard, &GuardContext {
        action,
        body,
        bind_stmt: &bind_stmt,
        action_config: config,
    })
}

/// Configuration for catchall binding generation.
#[derive(Clone, Copy, Debug)]
pub struct CatchallBindingConfig {
    /// Whether to consume the error (owned) or borrow it (ref)
    pub consume: bool,
}

impl CatchallBindingConfig {
    /// Create config for catch handlers (consume the error)
    pub fn catch() -> Self {
        Self { consume: true }
    }

    /// Create config for throw/inspect handlers (borrow the error)
    pub fn borrow() -> Self {
        Self { consume: false }
    }
}

/// Generated bindings for catchall handlers.
pub struct CatchallBindings {
    /// Main binding statement
    pub bind_stmt: TokenStream,
    /// Reference binding for guard condition check
    pub ref_bind: TokenStream,
    /// Binding for action execution (with allow(unused_variables))
    pub action_bind: TokenStream,
}

/// Generate bindings for catchall handlers.
///
/// Returns separate bindings for:
/// - `bind_stmt` - Main binding (before guard evaluation)
/// - `ref_bind` - Reference binding for guard condition
/// - `action_bind` - Binding for action execution
///
/// All bindings include `#[allow(unused_variables)]` because the binding may not
/// be referenced in the guard condition. For example, `throw e when true { ... }`
/// creates a binding `e` that isn't used in the guard `true`, which would otherwise
/// trigger an unused variable warning.
pub fn gen_catchall_bindings(binding: &Ident, config: CatchallBindingConfig) -> CatchallBindings {
    let binding_str = binding.to_string();

    let bind_stmt = if binding_str == "_" {
        quote! {}
    } else if config.consume {
        quote! { #[allow(unused_variables)] let #binding: ::handle_this::Handled = __err; }
    } else {
        quote! { #[allow(unused_variables)] let #binding: &::handle_this::Handled = &__err; }
    };

    let ref_bind = if binding_str == "_" {
        quote! {}
    } else {
        quote! { #[allow(unused_variables)] let #binding: &::handle_this::Handled = &__err; }
    };

    let action_bind = if binding_str == "_" {
        quote! {}
    } else if config.consume {
        quote! { #[allow(unused_variables)] let #binding: ::handle_this::Handled = __err; }
    } else {
        quote! { #[allow(unused_variables)] let #binding: &::handle_this::Handled = &__err; }
    };

    CatchallBindings {
        bind_stmt,
        ref_bind,
        action_bind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_check_root_downcast() {
        let type_path = quote! { std::io::Error };
        let binding = syn::Ident::new("e", proc_macro2::Span::call_site());
        let inner = quote! { return Ok(42); };

        let code = wrap_with_type_check(
            ChainVariant::Root,
            &type_path,
            &binding,
            &inner,
            TypeCheckMode::DowncastRoot,
        );
        let code_str = code.to_string();
        assert!(code_str.contains("downcast_ref"));
    }

    #[test]
    fn test_type_check_root_chain() {
        let type_path = quote! { std::io::Error };
        let binding = syn::Ident::new("e", proc_macro2::Span::call_site());
        let inner = quote! { return Ok(42); };

        let code = wrap_with_type_check(
            ChainVariant::Root,
            &type_path,
            &binding,
            &inner,
            TypeCheckMode::ChainRoot,
        );
        let code_str = code.to_string();
        assert!(code_str.contains("chain_any"));
    }

    #[test]
    fn test_type_check_all() {
        let type_path = quote! { std::io::Error };
        let binding = syn::Ident::new("errors", proc_macro2::Span::call_site());
        let inner = quote! { println!("{:?}", errors); };

        let code = wrap_with_type_check(
            ChainVariant::All,
            &type_path,
            &binding,
            &inner,
            TypeCheckMode::ChainRoot,
        );
        let code_str = code.to_string();
        assert!(code_str.contains("chain_all"));
        assert!(code_str.contains("is_empty"));
    }

    #[test]
    fn test_catchall_bindings_consume() {
        let binding = syn::Ident::new("e", proc_macro2::Span::call_site());
        let bindings = gen_catchall_bindings(&binding, CatchallBindingConfig::catch());

        let bind_str = bindings.bind_stmt.to_string();
        assert!(bind_str.contains("let e : :: handle_this :: Handled = __err"));
        assert!(!bind_str.contains("&"));
    }

    #[test]
    fn test_catchall_bindings_borrow() {
        let binding = syn::Ident::new("e", proc_macro2::Span::call_site());
        let bindings = gen_catchall_bindings(&binding, CatchallBindingConfig::borrow());

        let bind_str = bindings.bind_stmt.to_string();
        assert!(bind_str.contains("& :: handle_this :: Handled"));
    }

    #[test]
    fn test_catchall_bindings_underscore() {
        let binding = syn::Ident::new("_", proc_macro2::Span::call_site());
        let bindings = gen_catchall_bindings(&binding, CatchallBindingConfig::catch());

        assert!(bindings.bind_stmt.is_empty());
        assert!(bindings.ref_bind.is_empty());
        assert!(bindings.action_bind.is_empty());
    }
}
