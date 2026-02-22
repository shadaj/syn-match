//! Pattern match `syn` expressions with quote-like syntax and `#` bindings.

use std::collections::HashMap;

use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Group, Ident, Span, TokenTree};
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

struct Branch {
    pattern: proc_macro2::TokenStream,
    body: Expr,
}

impl Parse for Branch {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let pattern = parse_pattern_tokens(input)?;
        input.parse::<Token![=>]>()?;
        let body: Expr = input.parse()?;
        Ok(Self { pattern, body })
    }
}

struct MatchExprInput {
    value: Expr,
    branches: Punctuated<Branch, Token![,]>,
}

impl Parse for MatchExprInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let value = input.parse::<Expr>()?;
        input.parse::<Token![,]>()?;

        let content;
        syn::braced!(content in input);
        let branches = Punctuated::<Branch, Token![,]>::parse_terminated(&content)?;

        Ok(Self { value, branches })
    }
}

fn parse_pattern_tokens(input: ParseStream) -> syn::Result<proc_macro2::TokenStream> {
    let mut tokens = proc_macro2::TokenStream::new();
    while !input.peek(Token![=>]) {
        let tt: proc_macro2::TokenTree = input.parse()?;
        tokens.extend(std::iter::once(tt));
    }
    Ok(tokens)
}

fn is_top_level_binding(pattern: &proc_macro2::TokenStream) -> bool {
    let mut iter = pattern.clone().into_iter();
    matches!(
        (iter.next(), iter.next(), iter.next()),
        (Some(TokenTree::Punct(p)), Some(TokenTree::Ident(_)), None) if p.as_char() == '#'
    )
}

/// Match a `syn::Expr` against pattern branches that use `#` bindings to grab subexpressions.
///
/// At least one branch must start with a top-level binding (for example `#lhs + #rhs`); this
/// signals to the macro that you intend to destructure the expression rather than rely solely on
/// wildcards.
///
/// # Example
///
/// ```rust
/// use syn_match::match_expr;
///
/// let expr = syn::parse_str("1 + 2").unwrap();
/// let (lhs, rhs) = match_expr!(
///     expr,
///     { #lhs + #rhs => (lhs.clone(), rhs.clone()), #_ => panic!("unexpected pattern") }
/// );
/// assert!(matches!(lhs, syn::Expr::Lit(_)));
/// assert!(matches!(rhs, syn::Expr::Lit(_)));
/// ```
///
/// ```compile_fail
/// use syn_match::match_expr;
///
/// let expr = syn::parse_str("1 + 2").unwrap();
/// let _ = match_expr!(
///     expr,
///     { #(lhs: Lit) + #rhs => (lhs.clone(), rhs.clone()) }
/// );
/// ```
#[proc_macro]
pub fn match_expr(input: TokenStream) -> TokenStream {
    match_expr_core(input.into()).into()
}

fn match_expr_core(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let input: MatchExprInput = syn::parse2(input).unwrap();
    let MatchExprInput { value, branches } = input;
    if !branches
        .iter()
        .any(|branch| is_top_level_binding(&branch.pattern))
    {
        return quote! {
            compile_error!("match_expr! requires at least one branch with a top-level binding")
        };
    }
    let mut counter = 0;

    let match_branches: Vec<_> = branches
        .iter()
        .map(|branch| {
            let (replaced, binding_variants) = replace_pattern_idents(branch.pattern.clone());
            let parsed_pattern =
                syn::parse2::<Expr>(replaced).expect("failed to parse pattern expr");
            let body = &branch.body;
            let mut conditions = Vec::new();
            generate_matcher(
                &parsed_pattern,
                quote! { __match_expr_value },
                &mut conditions,
                &mut counter,
                &binding_variants,
            );

            let mut matcher = quote! { break { #body }; };
            while let Some(cond) = conditions.pop() {
                matcher = cond.wrap(matcher);
            }

            quote! { { #matcher } }
        })
        .collect();

    quote! {
        {
            let __match_expr_value_owned = #value;
            let __match_expr_value = &__match_expr_value_owned;
            #[allow(unreachable_code, clippy::diverging_sub_expression)]
            loop {
                #(#match_branches)*
                unreachable!("match_expr! pattern did not match any branch");
            }
        }
    }
}

type BindingVariants = HashMap<String, Ident>;

struct BindingWithVariant {
    name: Ident,
    _colon: Token![:],
    variant: Ident,
}

impl Parse for BindingWithVariant {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            name: input.parse()?,
            _colon: input.parse()?,
            variant: input.parse()?,
        })
    }
}

fn replace_pattern_idents(
    tokens: proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, BindingVariants) {
    let mut output = Vec::new();
    let mut binding_variants = BindingVariants::new();
    let mut iter = tokens.into_iter().peekable();
    while let Some(tt) = iter.next() {
        match tt {
            TokenTree::Punct(ref p) if p.as_char() == '#' => match iter.next() {
                Some(TokenTree::Ident(ident)) => {
                    let new_ident = format_ident!("PATTERN_{}", ident, span = ident.span());
                    output.push(TokenTree::Ident(new_ident));
                }
                Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
                    let parsed = syn::parse2::<BindingWithVariant>(group.stream());
                    if let Ok(binding) = parsed {
                        let new_ident =
                            format_ident!("PATTERN_{}", binding.name, span = binding.name.span());
                        binding_variants.insert(
                            binding.name.to_string(),
                            Ident::new(&binding.variant.to_string(), binding.variant.span()),
                        );
                        output.push(TokenTree::Ident(new_ident));
                    } else {
                        output.push(TokenTree::Punct(p.clone()));
                        output.push(TokenTree::Group(group));
                    }
                }
                Some(other) => {
                    output.push(TokenTree::Punct(p.clone()));
                    output.push(other);
                }
                None => output.push(TokenTree::Punct(p.clone())),
            },
            TokenTree::Group(group) => {
                let (inner, variants) = replace_pattern_idents(group.stream());
                binding_variants.extend(variants);
                let mut new_group = Group::new(group.delimiter(), inner);
                new_group.set_span(group.span());
                output.push(TokenTree::Group(new_group));
            }
            other => output.push(other),
        }
    }

    (output.into_iter().collect(), binding_variants)
}

fn generate_matcher(
    pattern: &Expr,
    target: proc_macro2::TokenStream,
    conditions: &mut Vec<Condition>,
    counter: &mut usize,
    binding_variants: &BindingVariants,
) {
    match pattern {
        Expr::Array(_) => todo!("matching for Expr::Array is not yet implemented"),
        Expr::Assign(_) => todo!("matching for Expr::Assign is not yet implemented"),
        Expr::Async(_) => todo!("matching for Expr::Async is not yet implemented"),
        Expr::Await(_) => todo!("matching for Expr::Await is not yet implemented"),
        Expr::Path(path) => {
            if let Some(binding) = binding_name(path) {
                if let Some(variant) = binding_variants.get(&binding.to_string()) {
                    let tmp = fresh_ident("__match_expr_variant", counter);
                    conditions.push(Condition::IfLet {
                        pat: quote! { &syn::Expr::#variant(ref #tmp) },
                        expr: target.clone(),
                    });
                    conditions.push(Condition::Let {
                        pat: binding,
                        expr: quote! { #tmp },
                    });
                } else {
                    conditions.push(Condition::Let {
                        pat: binding,
                        expr: target,
                    });
                }
            } else {
                todo!("matching for non-binding paths is not yet implemented");
            }
        }
        Expr::Binary(bin) => {
            let tmp = fresh_ident("__match_expr_binary", counter);

            conditions.push(Condition::IfLet {
                pat: quote! { &syn::Expr::Binary(ref #tmp) },
                expr: target,
            });

            gen_attributes_matcher(&bin.attrs, quote! { #tmp.attrs }, conditions);

            match bin.op {
                syn::BinOp::Add(_) => conditions.push(Condition::If {
                    cond: quote! { matches!(#tmp.op, syn::BinOp::Add(_)) },
                }),
                _ => todo!("matching for binary operators other than `+` is not yet implemented"),
            }

            generate_matcher(
                &bin.left,
                quote! { &*#tmp.left },
                conditions,
                counter,
                binding_variants,
            );
            generate_matcher(
                &bin.right,
                quote! { &*#tmp.right },
                conditions,
                counter,
                binding_variants,
            );
        }
        Expr::Block(_) => todo!("matching for Expr::Block is not yet implemented"),
        Expr::Break(_) => todo!("matching for Expr::Break is not yet implemented"),
        Expr::Call(_) => todo!("matching for Expr::Call is not yet implemented"),
        Expr::Cast(_) => todo!("matching for Expr::Cast is not yet implemented"),
        Expr::Closure(_) => todo!("matching for Expr::Closure is not yet implemented"),
        Expr::Const(_) => todo!("matching for Expr::Const is not yet implemented"),
        Expr::Continue(_) => todo!("matching for Expr::Continue is not yet implemented"),
        Expr::Field(_) => todo!("matching for Expr::Field is not yet implemented"),
        Expr::ForLoop(_) => todo!("matching for Expr::ForLoop is not yet implemented"),
        Expr::Group(_) => todo!("matching for Expr::Group is not yet implemented"),
        Expr::If(_) => todo!("matching for Expr::If is not yet implemented"),
        Expr::Index(_) => todo!("matching for Expr::Index is not yet implemented"),
        Expr::Infer(_) => todo!("matching for Expr::Infer is not yet implemented"),
        Expr::Let(_) => todo!("matching for Expr::Let is not yet implemented"),
        Expr::Lit(_) => todo!("matching for Expr::Lit is not yet implemented"),
        Expr::Loop(_) => todo!("matching for Expr::Loop is not yet implemented"),
        Expr::Macro(_) => todo!("matching for Expr::Macro is not yet implemented"),
        Expr::Match(_) => todo!("matching for Expr::Match is not yet implemented"),
        Expr::MethodCall(_) => todo!("matching for Expr::MethodCall is not yet implemented"),
        Expr::Paren(_) => todo!("matching for Expr::Paren is not yet implemented"),
        Expr::Range(_) => todo!("matching for Expr::Range is not yet implemented"),
        Expr::RawAddr(_) => todo!("matching for Expr::RawAddr is not yet implemented"),
        Expr::Reference(_) => todo!("matching for Expr::Reference is not yet implemented"),
        Expr::Repeat(_) => todo!("matching for Expr::Repeat is not yet implemented"),
        Expr::Return(_) => todo!("matching for Expr::Return is not yet implemented"),
        Expr::Struct(_) => todo!("matching for Expr::Struct is not yet implemented"),
        Expr::Try(_) => todo!("matching for Expr::Try is not yet implemented"),
        Expr::TryBlock(_) => todo!("matching for Expr::TryBlock is not yet implemented"),
        Expr::Tuple(_) => todo!("matching for Expr::Tuple is not yet implemented"),
        Expr::Unary(_) => todo!("matching for Expr::Unary is not yet implemented"),
        Expr::Unsafe(_) => todo!("matching for Expr::Unsafe is not yet implemented"),
        Expr::Verbatim(_) => todo!("matching for Expr::Verbatim is not yet implemented"),
        Expr::While(_) => todo!("matching for Expr::While is not yet implemented"),
        Expr::Yield(_) => todo!("matching for Expr::Yield is not yet implemented"),
        _ => todo!("matching for this kind of expression is not yet implemented"),
    }
}

fn gen_attributes_matcher(
    attrs: &[Attribute],
    target: proc_macro2::TokenStream,
    conditions: &mut Vec<Condition>,
) {
    if attrs.is_empty() {
        conditions.push(Condition::If {
            cond: quote! { #target.is_empty() },
        });
    } else {
        todo!("matching for attributes is not yet implemented");
    }
}

fn binding_name(path: &syn::ExprPath) -> Option<Ident> {
    let ident = path.path.get_ident()?;
    let ident_str = ident.to_string();
    ident_str
        .strip_prefix("PATTERN_")
        .map(|rest| Ident::new(rest, ident.span()))
}

fn fresh_ident(prefix: &str, counter: &mut usize) -> Ident {
    let current = *counter;
    *counter += 1;
    Ident::new(&format!("{prefix}_{current}"), Span::mixed_site())
}

enum Condition {
    IfLet {
        pat: proc_macro2::TokenStream,
        expr: proc_macro2::TokenStream,
    },
    If {
        cond: proc_macro2::TokenStream,
    },
    Let {
        pat: syn::Ident,
        expr: proc_macro2::TokenStream,
    },
}

impl Condition {
    fn wrap(self, inner: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        match self {
            Condition::IfLet { pat, expr } => quote! {
                if let #pat = #expr {
                    #inner
                }
            },
            Condition::If { cond } => quote! {
                if #cond {
                    #inner
                }
            },
            Condition::Let { pat, expr } => quote! {
                let #pat = #expr;
                #inner
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use std::str::FromStr;

    fn snapshot_macro(input: &str) -> String {
        // Call the proc macro function directly with the quoted input and stringify the expansion.
        let ts = proc_macro2::TokenStream::from_str(input).expect("failed to parse input");
        let expanded = match_expr_core(ts);
        prettyplease::unparse(&syn::parse_quote! {
            fn test() {
                #expanded
            }
        })
    }

    #[test]
    fn snapshot_simple_add() {
        assert_snapshot!(
            "match_expr_add",
            snapshot_macro(
                r#"
                syn::parse_str("1 + 2").unwrap(),
                {
                    #lhs + #rhs => (lhs.clone(), rhs.clone()),
                    #_ => panic!("no match"),
                }
                "#
            )
        );
    }

    #[test]
    fn snapshot_variant_binding() {
        assert_snapshot!(
            "match_expr_lit_variant",
            snapshot_macro(
                r#"
                syn::parse_str("1 + 2").unwrap(),
                {
                    #(lhs: Lit) + #_ => lhs.lit.clone(),
                    #_ => panic!("no match"),
                }
                "#
            )
        );
    }
}
