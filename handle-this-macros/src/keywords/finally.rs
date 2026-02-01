//! Finally keyword - cleanup that always runs.
//!
//! Syntax: `finally { cleanup_code }`
//!
//! The finally block is inlined (not wrapped in closures) to allow
//! mutable borrows to work naturally across try/finally blocks.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::ParseStream;
use syn::{Result, braced};

use super::parse_keyword;

/// Parse a finally clause body.
pub fn parse(input: ParseStream) -> Result<TokenStream> {
    parse_keyword(input, "finally")?;
    let content;
    braced!(content in input);
    content.parse()
}

/// Wrap code with finally block.
///
/// The finally block runs regardless of success or failure.
/// Uses inline code (not closures) to allow mutable borrows.
///
/// Note: This works for both sync and async try blocks because the
/// inner code has already been awaited before finally runs.
pub fn wrap(inner: TokenStream, finally_body: &TokenStream) -> TokenStream {
    quote! {
        {
            #[allow(unreachable_code)]
            let __finally_result = { #inner };
            { #finally_body }
            __finally_result
        }
    }
}
