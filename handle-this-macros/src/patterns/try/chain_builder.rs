//! Chain builder for nested handler chains.
//!
//! Extracts the common pattern of building nested if-else chains for handlers.
//! Used by sync.rs for both "direct mode" (returns values) and "result mode"
//! (returns Result, propagates unmatched errors).

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::keywords::{ChainVariant, Guard};

/// Generate a typed handler check with guard support.
///
/// This is the core function used by all typed handlers (catch, throw, try catch).
/// It generates:
/// ```ignore
/// if let Some(__typed_err) = TYPE_CHECK {
///     let binding = __typed_err;
///     GUARD_CHECK {
///         body
///     } else {
///         else_branch
///     }
/// } else {
///     else_branch
/// }
/// ```
pub fn gen_typed_with_guard(
    type_check: &TokenStream,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    match guard {
        Some(Guard::When(cond)) => quote! {
            if let ::core::option::Option::Some(__typed_err) = #type_check {
                let #binding = __typed_err;
                if #cond {
                    #body
                } else {
                    #else_branch
                }
            } else {
                #else_branch
            }
        },
        Some(Guard::Match { expr, arms }) => quote! {
            if let ::core::option::Option::Some(__typed_err) = #type_check {
                let #binding = __typed_err;
                match #expr { #arms }
            } else {
                #else_branch
            }
        },
        None => quote! {
            if let ::core::option::Option::Some(__typed_err) = #type_check {
                let #binding = __typed_err;
                #body
            } else {
                #else_branch
            }
        },
    }
}

/// Generate a catchall handler (no type check) with guard support.
///
/// For when there's no type filter, the binding receives the error directly.
pub fn gen_catchall_with_guard(
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    let binding_str = binding.to_string();
    let bind_stmt = if binding_str == "_" {
        quote! {}
    } else {
        quote! { let #binding = __err; }
    };

    match guard {
        Some(Guard::When(cond)) => {
            let ref_bind = if binding_str == "_" {
                quote! {}
            } else {
                quote! { let #binding = &__err; }
            };
            quote! {
                {
                    #ref_bind
                    if #cond {
                        #bind_stmt
                        #body
                    } else {
                        #else_branch
                    }
                }
            }
        }
        Some(Guard::Match { expr, arms }) => quote! {
            {
                #bind_stmt
                match #expr { #arms }
            }
        },
        None => quote! {
            {
                #bind_stmt
                #body
            }
        },
    }
}

/// Generate an "all" variant handler (collects Vec<&Type>).
pub fn gen_all_variant_with_guard(
    type_path: &TokenStream,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    match guard {
        Some(Guard::When(cond)) => quote! {
            {
                let #binding: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                if !#binding.is_empty() && #cond {
                    #body
                } else {
                    #else_branch
                }
            }
        },
        Some(Guard::Match { expr, arms }) => quote! {
            {
                let #binding: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                if !#binding.is_empty() {
                    match #expr { #arms }
                } else {
                    #else_branch
                }
            }
        },
        None => quote! {
            {
                let #binding: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                if !#binding.is_empty() {
                    #body
                } else {
                    #else_branch
                }
            }
        },
    }
}

/// Generate a complete handler for a given variant.
///
/// Dispatches to the appropriate generator based on variant and type.
pub fn gen_handler(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    match (type_path, variant) {
        (None, ChainVariant::Root) => {
            gen_catchall_with_guard(binding, guard, body, else_branch)
        }
        (Some(type_path), ChainVariant::Root) => {
            let type_check = quote! { __err.downcast_ref::<#type_path>() };
            gen_typed_with_guard(&type_check, binding, guard, body, else_branch)
        }
        (Some(type_path), ChainVariant::Any) => {
            let type_check = quote! { __err.chain_any::<#type_path>() };
            gen_typed_with_guard(&type_check, binding, guard, body, else_branch)
        }
        (Some(type_path), ChainVariant::All) => {
            gen_all_variant_with_guard(type_path, binding, guard, body, else_branch)
        }
        (None, _) => {
            syn::Error::new(binding.span(), "any/all requires a type").to_compile_error()
        }
    }
}

/// Generate an inspect handler (side effects only, always continues to else_branch).
pub fn gen_inspect_handler(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    match (type_path, variant) {
        (None, ChainVariant::Root) => {
            // Untyped inspect - always runs side effect
            let binding_str = binding.to_string();
            let bind_stmt = if binding_str == "_" {
                quote! {}
            } else {
                quote! { let #binding = &__err; }
            };

            match guard {
                Some(Guard::When(cond)) => quote! {
                    {
                        #bind_stmt
                        if #cond {
                            let _ = { #body };
                        }
                        #else_branch
                    }
                },
                Some(Guard::Match { expr, arms }) => quote! {
                    {
                        #bind_stmt
                        let _ = match #expr { #arms };
                        #else_branch
                    }
                },
                None => quote! {
                    {
                        #bind_stmt
                        let _ = { #body };
                        #else_branch
                    }
                },
            }
        }
        (Some(type_path), ChainVariant::Root) => {
            let type_check = quote! { __err.downcast_ref::<#type_path>() };
            gen_typed_inspect(&type_check, binding, guard, body, else_branch)
        }
        (Some(type_path), ChainVariant::Any) => {
            let type_check = quote! { __err.chain_any::<#type_path>() };
            gen_typed_inspect(&type_check, binding, guard, body, else_branch)
        }
        (Some(type_path), ChainVariant::All) => {
            match guard {
                Some(Guard::When(cond)) => quote! {
                    {
                        let #binding: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                        if !#binding.is_empty() && #cond {
                            let _ = { #body };
                        }
                        #else_branch
                    }
                },
                Some(Guard::Match { expr, arms }) => quote! {
                    {
                        let #binding: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                        if !#binding.is_empty() {
                            let _ = match #expr { #arms };
                        }
                        #else_branch
                    }
                },
                None => quote! {
                    {
                        let #binding: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                        if !#binding.is_empty() {
                            let _ = { #body };
                        }
                        #else_branch
                    }
                },
            }
        }
        (None, _) => {
            syn::Error::new(binding.span(), "inspect any/all requires a type").to_compile_error()
        }
    }
}

/// Generate a typed inspect handler.
fn gen_typed_inspect(
    type_check: &TokenStream,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    match guard {
        Some(Guard::When(cond)) => quote! {
            {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    if #cond {
                        let _ = { #body };
                    }
                }
                #else_branch
            }
        },
        Some(Guard::Match { expr, arms }) => quote! {
            {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    let _ = match #expr { #arms };
                }
                #else_branch
            }
        },
        None => quote! {
            {
                if let ::core::option::Option::Some(__typed_err) = #type_check {
                    let #binding = __typed_err;
                    let _ = { #body };
                }
                #else_branch
            }
        },
    }
}
