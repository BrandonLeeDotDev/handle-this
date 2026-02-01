//! Unified clause parsing for catch, throw, inspect, try_catch.
//!
//! All handler clauses share the same basic structure:
//! - Optional chain variant (any/all)
//! - Optional type filter
//! - Binding identifier
//! - Optional guard (when/match)
//! - Body expression
//!
//! This module provides a unified parsing function that handles
//! all the variations, controlled by `ClauseConfig`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::ParseStream;
use syn::{Ident, Result};

use super::{ChainVariant, Guard, is_lowercase_ident, peek_keyword};
use super::parsing::{self, parse_chain_variant, parse_type_path, parse_binding, parse_optional_guard};

/// Configuration for clause parsing behavior.
#[derive(Debug, Clone, Copy)]
pub struct ClauseConfig {
    /// The keyword name for error messages ("catch", "throw", "inspect", "try catch")
    pub keyword: &'static str,
    /// Allow `keyword { }` without binding (catch, throw allow this; inspect doesn't)
    pub allow_no_binding_catchall: bool,
    /// Binding is optional even for typed clauses (throw uses Option<Ident>)
    pub binding_optional: bool,
}

impl ClauseConfig {
    /// Config for catch clauses
    pub fn catch() -> Self {
        Self {
            keyword: "catch",
            allow_no_binding_catchall: true,
            binding_optional: false,
        }
    }

    /// Config for throw clauses
    pub fn throw() -> Self {
        Self {
            keyword: "throw",
            allow_no_binding_catchall: true,
            binding_optional: true,
        }
    }

    /// Config for inspect clauses
    pub fn inspect() -> Self {
        Self {
            keyword: "inspect",
            allow_no_binding_catchall: false,
            binding_optional: false,
        }
    }

    /// Config for try_catch clauses
    pub fn try_catch() -> Self {
        Self {
            keyword: "try catch",
            allow_no_binding_catchall: true,
            binding_optional: false,
        }
    }
}

/// A parsed clause (generic over all handler types).
#[derive(Debug, Clone)]
pub struct ParsedClause {
    /// Chain variant: Root, Any, or All
    pub variant: ChainVariant,
    /// Type path (None for catch-all)
    pub type_path: Option<TokenStream>,
    /// Error binding identifier (None if no binding specified and config allows)
    pub binding: Option<Ident>,
    /// Optional guard (when or match)
    pub guard: Option<Guard>,
    /// Body expression
    pub body: TokenStream,
}

/// Parse a handler clause after the keyword has been consumed.
///
/// The keyword should already be parsed; this function handles:
/// - Chain variant (any/all)
/// - Catch-all patterns
/// - Typed patterns with binding
/// - Guards
/// - Body
pub fn parse_clause(
    input: ParseStream,
    _keyword_span: proc_macro2::Span,
    config: ClauseConfig,
) -> Result<ParsedClause> {
    // Check for chain variant (any/all)
    let variant = parse_chain_variant(input)?;

    // any/all require a type path - check for common mistake of `catch any e { }`
    if variant != ChainVariant::Root {
        let fork = input.fork();
        if let Ok(ident) = fork.parse::<Ident>() {
            if is_lowercase_ident(&ident) && (parsing::peek_brace(&fork) || peek_keyword(&fork, "when") || fork.is_empty()) {
                let kw = config.keyword;
                let (variant_name, example) = if variant == ChainVariant::Any {
                    ("any", format!("{} any Type(e) {{ ... }}", kw))
                } else {
                    ("all", format!("{} all Type |errors| {{ ... }}", kw))
                };
                return Err(syn::Error::new(
                    ident.span(),
                    format!("`{}` requires a type: `{}`", variant_name, example),
                ));
            }
        }
    }

    // Check for catch-all: `{ }` without binding
    if variant == ChainVariant::Root && parsing::peek_brace(input) {
        if config.allow_no_binding_catchall {
            let body = parsing::parse_braced_body(input)?;
            return Ok(ParsedClause {
                variant,
                type_path: None,
                binding: None,
                guard: None,
                body,
            });
        } else {
            // Handler doesn't allow catch-all without binding (e.g., inspect)
            return Err(syn::Error::new(
                input.span(),
                format!("`{}` requires a binding: `{} e {{ ... }}`", config.keyword, config.keyword),
            ));
        }
    }

    // Check for underscore binding: `_ { }` or `_ when ... { }`
    if variant == ChainVariant::Root && input.peek(syn::Token![_]) {
        let fork = input.fork();
        fork.parse::<syn::Token![_]>().ok();
        if parsing::peek_brace(&fork) || peek_keyword(&fork, "when") {
            input.parse::<syn::Token![_]>()?; // consume _
            let binding = parsing::underscore_ident();

            // Parse optional guard
            let guard = parse_optional_guard(input)?;

            // Parse body
            let body = parsing::parse_braced_body(input)?;

            return Ok(ParsedClause {
                variant,
                type_path: None,
                binding: Some(binding),
                guard,
                body,
            });
        }
    }

    // Check for lowercase ident binding: `e { }` or `e when ... { }`
    let fork = input.fork();
    if let Ok(ident) = fork.parse::<Ident>() {
        if is_lowercase_ident(&ident) && variant == ChainVariant::Root {
            if parsing::peek_brace(&fork) || peek_keyword(&fork, "when") {
                input.parse::<Ident>()?; // consume binding

                // Check for reserved internal names
                let name = ident.to_string();
                if parsing::is_reserved_binding(&name) {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("`{}` is reserved for internal use; choose a different binding name", name),
                    ));
                }

                // Parse optional guard
                let guard = parse_optional_guard(input)?;

                // Parse body
                let body = parsing::parse_braced_body(input)?;

                return Ok(ParsedClause {
                    variant,
                    type_path: None,
                    binding: Some(ident),
                    guard,
                    body,
                });
            } else if peek_keyword(&fork, "if") {
                // Common mistake: using `if` instead of `when`
                return Err(syn::Error::new(
                    fork.span(),
                    "use `when` for guards, not `if`: `catch e when condition { ... }`",
                ));
            } else if fork.is_empty() {
                // Looks like `catch e` at end of input - missing body
                return Err(syn::Error::new(
                    ident.span(),
                    "missing body after binding, expected `{ ... }`",
                ));
            }
        }
    }

    // Typed clause: Type(binding) or Type |binding| (for all variant)
    // Also supports shorthand: Type { } without binding
    let type_path = parse_type_path(input)?;

    // Check for binding
    let binding = if input.peek(syn::token::Paren) || input.peek(syn::Token![|]) {
        Some(parse_binding(input, variant)?)
    } else if config.binding_optional {
        // Shorthand - no explicit binding
        None
    } else {
        // Shorthand - use underscore as implicit binding
        Some(parsing::underscore_ident())
    };

    // Parse optional guard (when or match)
    let guard = parse_optional_guard(input)?;

    // For match clause, the arms are the body - no separate braces needed
    let body = if matches!(guard, Some(Guard::Match { .. })) {
        quote! {}
    } else {
        parsing::parse_braced_body(input)?
    };

    Ok(ParsedClause {
        variant,
        type_path: Some(type_path),
        binding,
        guard,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn parse_test(tokens: proc_macro2::TokenStream, config: ClauseConfig) -> Result<ParsedClause> {
        syn::parse::Parser::parse2(
            |input: ParseStream| {
                parse_clause(input, proc_macro2::Span::call_site(), config)
            },
            tokens,
        )
    }

    #[test]
    fn test_catch_no_binding() {
        let clause = parse_test(parse_quote! { { 42 } }, ClauseConfig::catch()).unwrap();
        assert!(clause.type_path.is_none());
        assert!(clause.binding.is_none());
    }

    #[test]
    fn test_catch_with_binding() {
        let clause = parse_test(parse_quote! { e { 42 } }, ClauseConfig::catch()).unwrap();
        assert!(clause.type_path.is_none());
        assert_eq!(clause.binding.unwrap().to_string(), "e");
    }

    #[test]
    fn test_typed_catch() {
        let clause = parse_test(
            parse_quote! { std::io::Error(e) { 42 } },
            ClauseConfig::catch(),
        ).unwrap();
        assert!(clause.type_path.is_some());
        assert_eq!(clause.binding.unwrap().to_string(), "e");
    }

    #[test]
    fn test_typed_shorthand() {
        let clause = parse_test(
            parse_quote! { std::io::Error { 42 } },
            ClauseConfig::catch(),
        ).unwrap();
        assert!(clause.type_path.is_some());
        // Shorthand gets underscore binding
        assert_eq!(clause.binding.unwrap().to_string(), "_");
    }

    #[test]
    fn test_throw_no_binding() {
        let clause = parse_test(parse_quote! { { "error" } }, ClauseConfig::throw()).unwrap();
        assert!(clause.binding.is_none());
    }

    #[test]
    fn test_inspect_requires_binding() {
        // inspect doesn't allow `{ }` without binding, so this parses as typed
        let result = parse_test(parse_quote! { { log(); } }, ClauseConfig::inspect());
        // This will fail because `{` is not a valid type path start
        assert!(result.is_err());
    }
}
