use proc_macro2::Span;
use syn_match::match_expr;

#[test]
fn matches_add_pattern() {
    let (left_is_lit, right_is_lit) = match_expr!(
        syn::parse_str("1 + 2").unwrap(),
        {
            #lhs + #rhs => {
                (matches!(lhs, &syn::Expr::Lit(_)), matches!(rhs, &syn::Expr::Lit(_)))
            },
            #_ => {
                panic!("unexpected pattern")
            }
        }
    );

    assert!(left_is_lit);
    assert!(right_is_lit);
}

#[test]
fn binds_variant_field() {
    let matcher = |x| {
        match_expr!(
            x,
            {
                #(lhs: Lit) + #_rhs => {
                    Some(lhs.lit.clone())
                },
                #_ => {
                    None
                }
            }
        )
    };

    assert_eq!(
        matcher(syn::parse_str("1 + 2").unwrap()),
        Some(syn::Lit::Int(syn::LitInt::new("1", Span::call_site())))
    );
    assert_eq!(
        matcher(syn::parse_str("\"hello\" + 2").unwrap()),
        Some(syn::Lit::Str(syn::LitStr::new("hello", Span::call_site())))
    );
    assert_eq!(matcher(syn::parse_str("not_lit + 2").unwrap()), None);
}

#[test]
fn continue_inside_arm() {
    let matcher = |x| {
        match_expr!(
            x,
            {
                #(lhs: Lit) + #_rhs => {
                    if let syn::Lit::Int(int) = &lhs.lit {
                        int.base10_parse::<i32>().ok()
                    } else {
                        None
                    }
                },
                #_ => {
                    None
                }
            }
        )
    };

    assert_eq!(matcher(syn::parse_str("1 + 2").unwrap()), Some(1));
    assert_eq!(matcher(syn::parse_str("not_lit + 2").unwrap()), None);
}
