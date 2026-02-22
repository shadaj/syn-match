# syn-match

Reverse-quoting for `syn`: describe the shape of an expression using a quasi-quoted pattern and bind pieces with `#` so you can destructure without manually walking the AST.

## Usage
Add this library to your `Cargo.toml`:

```toml
syn-match = "0.1"
```

- Write branches that look like the expression you expect, but insert `#name` to bind subexpressions.
- If you need to capture the inner variant of a node (for example a literal), use `#(name: Variant)`; it binds `name` to the contents of `syn::Expr::Variant`.
- Include at least one catch-all branch that is just a top-level binding (e.g. `#_`) so the macro can fail cleanly when nothing else matches.

```rust
use syn_match::match_expr;

fn split_add(expr: syn::Expr) -> Option<(syn::Expr, syn::Expr)> {
    match_expr!(
        expr,
        {
            #lhs + #rhs => Some((lhs.clone(), rhs.clone())),
            #_ => None
        }
    )
}
```

This will bind `lhs` and `rhs` to the left and right sides when the expression is an addition, and return `None` otherwise.

## What gets generated
`match_expr!` expands into a `loop` that tries each branch in order and `break`s with the first match. Every pattern is turned into a series of nested `if let`/`if` guards against the `syn::Expr` tree, and bindings become local `let` expressions. There is no matching logic at runtime beyond checking each branch.

For example, this invocation:

```rust
match_expr!(
    syn::parse_str("1 + 2").unwrap(),
    {
        #lhs + #rhs => (lhs.clone(), rhs.clone()),
        #_ => panic!("no match")
    }
)
```

expands to (simplified for readability):

```rust
{
    let __match_expr_value_owned = syn::parse_str("1 + 2").unwrap();
    let __match_expr_value = &__match_expr_value_owned;

    loop {
        if let &syn::Expr::Binary(ref __match_expr_binary_0) = __match_expr_value {
            if matches!(__match_expr_binary_0.op, syn::BinOp::Add(_)) {
                let lhs = &*__match_expr_binary_0.left;
                let rhs = &*__match_expr_binary_0.right;
                break { (lhs.clone(), rhs.clone()) };
            }
        }
        let _ = __match_expr_value;
        break { panic!("no match") };
    }
}
```
