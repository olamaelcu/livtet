//! DSL primitives for building tantivy [`UserInputAst`] trees.
//!
//! Every saved-search filter eventually lowers to a `UserInputAst`
//! that tantivy can parse. These helpers exist so the topical enums
//! don't reinvent the same literal / exists / range construction
//! rules — and so the escape rules stay consistent.

use tantivy_query_grammar::{
    Delimiter, Occur, UserInputAst, UserInputBound, UserInputLeaf, UserInputLiteral,
};

/// `field:value` literal (no surrounding quotes, single token).
pub fn field_term(field: &str, value: impl Into<String>) -> UserInputAst {
    let literal = UserInputLiteral {
        field_name: Some(field.to_string()),
        phrase: value.into(),
        delimiter: Delimiter::None,
        slop: 0,
        prefix: false,
    };
    UserInputAst::Leaf(Box::new(UserInputLeaf::Literal(literal)))
}

/// `field:"value"` literal — quoted, whole-phrase match.
pub fn field_term_exact(field: &str, value: impl Into<String>) -> UserInputAst {
    let literal = UserInputLiteral {
        field_name: Some(field.to_string()),
        phrase: value.into(),
        delimiter: Delimiter::DoubleQuotes,
        slop: 0,
        prefix: false,
    };
    UserInputAst::Leaf(Box::new(UserInputLeaf::Literal(literal)))
}

/// Bare term against a field — used when the caller wants the term
/// tokenised (escaping applied by tantivy's own normalizer). No
/// quotes. Equivalent to `field_term` but signals intent for hybrid
/// ID+text categorisation paths.
pub fn field_term_undelimited(field: &str, value: impl Into<String>) -> UserInputAst {
    field_term(field, value)
}

/// `field:{a b c}` set literal — tantivy matches any listed value.
pub fn field_term_exact_set(field: &str, values: impl IntoIterator<Item = String>) -> UserInputAst {
    let elements: Vec<String> = values.into_iter().collect();
    UserInputAst::Leaf(Box::new(UserInputLeaf::Set {
        field: Some(field.to_string()),
        elements,
    }))
}

/// `field:EXISTS` — used for `HasCover`, `HasNotes`, etc.
pub fn field_exists(field: impl Into<String>) -> UserInputAst {
    UserInputAst::Leaf(Box::new(UserInputLeaf::Exists {
        field: field.into(),
    }))
}

/// `field:[lower TO upper]` inclusive range.
pub fn range_inclusive(
    field: &str,
    lower: impl Into<String>,
    upper: impl Into<String>,
) -> UserInputAst {
    UserInputAst::Leaf(Box::new(UserInputLeaf::Range {
        field: Some(field.to_string()),
        lower: UserInputBound::Inclusive(lower.into()),
        upper: UserInputBound::Inclusive(upper.into()),
    }))
}

/// `(a AND b AND ...)` — every child MUST match.
pub fn and_clause(children: Vec<UserInputAst>) -> UserInputAst {
    let clause = children
        .into_iter()
        .map(|c| (Some(Occur::Must), c))
        .collect();
    UserInputAst::Clause(clause)
}

/// `(a OR b OR ...)` — at least one child should match.
pub fn or_clause(children: Vec<UserInputAst>) -> UserInputAst {
    let clause = children
        .into_iter()
        .map(|c| (Some(Occur::Should), c))
        .collect();
    UserInputAst::Clause(clause)
}

/// `NOT a` — exclude documents matching `child`.
pub fn must_not_clause(child: UserInputAst) -> UserInputAst {
    UserInputAst::Clause(vec![(Some(Occur::MustNot), child)])
}

/// Escape a term that will be emitted unquoted inside a tantivy
/// query. Tantivy's parser splits on whitespace and treats a small
/// set of characters (`+`, `-`, `!`, `(`, `)`, `{`, `}`, `[`, `]`,
/// `^`, `"`, `~`, `*`, `?`, `:`, `\`, `/`) as syntax. We backslash-
/// escape the lot so the value is taken literally.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let needs = matches!(
            ch,
            '\\' | '+'
                | '-'
                | '!'
                | '('
                | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '^'
                | '"'
                | '~'
                | '*'
                | '?'
                | ':'
                | '/'
                | ' '
                | '\t'
                | '\n'
                | '\r'
        );
        if needs {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_leaves_plain_text_unchanged() {
        assert_eq!(escape("hello"), "hello");
        assert_eq!(escape("abc123"), "abc123");
    }

    #[test]
    fn escape_backslash_escaping() {
        assert_eq!(escape("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_special_chars() {
        assert_eq!(escape("a+b"), "a\\+b");
        assert_eq!(escape("a-b"), "a\\-b");
        assert_eq!(escape("a!b"), "a\\!b");
        assert_eq!(escape("a(b"), "a\\(b");
        assert_eq!(escape("a)b"), "a\\)b");
        assert_eq!(escape("a{b"), "a\\{b");
        assert_eq!(escape("a}b"), "a\\}b");
        assert_eq!(escape("a[b"), "a\\[b");
        assert_eq!(escape("a]b"), "a\\]b");
        assert_eq!(escape("a^b"), "a\\^b");
        assert_eq!(escape("a\"b"), "a\\\"b");
        assert_eq!(escape("a~b"), "a\\~b");
        assert_eq!(escape("a*b"), "a\\*b");
        assert_eq!(escape("a?b"), "a\\?b");
        assert_eq!(escape("a:b"), "a\\:b");
        assert_eq!(escape("a/b"), "a\\/b");
    }

    #[test]
    fn escape_whitespace() {
        assert_eq!(escape("a b"), "a\\ b");
        assert_eq!(escape("a\tb"), "a\\\tb");
        assert_eq!(escape("a\nb"), "a\\\nb");
        assert_eq!(escape("a\rb"), "a\\\rb");
    }

    #[test]
    fn field_term_produces_literal() {
        let ast = field_term("title", "hello");
        match ast {
            UserInputAst::Leaf(leaf) => match *leaf {
                UserInputLeaf::Literal(lit) => {
                    assert_eq!(lit.field_name, Some("title".to_string()));
                    assert_eq!(lit.phrase, "hello");
                    assert_eq!(lit.delimiter, Delimiter::None);
                }
                _ => panic!("expected Literal"),
            },
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn field_term_exact_uses_double_quotes() {
        let ast = field_term_exact("title", "hello world");
        match ast {
            UserInputAst::Leaf(leaf) => match *leaf {
                UserInputLeaf::Literal(lit) => {
                    assert_eq!(lit.delimiter, Delimiter::DoubleQuotes);
                    assert_eq!(lit.phrase, "hello world");
                }
                _ => panic!("expected Literal"),
            },
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn field_exists_produces_exists_leaf() {
        let ast = field_exists("cover");
        match ast {
            UserInputAst::Leaf(leaf) => match *leaf {
                UserInputLeaf::Exists { field } => {
                    assert_eq!(field, "cover");
                }
                _ => panic!("expected Exists"),
            },
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn range_inclusive_produces_range() {
        let ast = range_inclusive("date", "2020", "2024");
        match ast {
            UserInputAst::Leaf(leaf) => match *leaf {
                UserInputLeaf::Range {
                    field,
                    lower,
                    upper,
                } => {
                    assert_eq!(field, Some("date".to_string()));
                    assert_eq!(lower, UserInputBound::Inclusive("2020".to_string()));
                    assert_eq!(upper, UserInputBound::Inclusive("2024".to_string()));
                }
                _ => panic!("expected Range"),
            },
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn and_clause_uses_must_occur() {
        let a = field_term("a", "1");
        let b = field_term("b", "2");
        let ast = and_clause(vec![a, b]);
        match ast {
            UserInputAst::Clause(children) => {
                assert_eq!(children.len(), 2);
                for (occur, _) in &children {
                    assert_eq!(*occur, Some(Occur::Must));
                }
            }
            _ => panic!("expected Clause"),
        }
    }

    #[test]
    fn or_clause_uses_should_occur() {
        let a = field_term("a", "1");
        let b = field_term("b", "2");
        let ast = or_clause(vec![a, b]);
        match ast {
            UserInputAst::Clause(children) => {
                assert_eq!(children.len(), 2);
                for (occur, _) in &children {
                    assert_eq!(*occur, Some(Occur::Should));
                }
            }
            _ => panic!("expected Clause"),
        }
    }

    #[test]
    fn must_not_clause_uses_must_not_occur() {
        let child = field_term("a", "1");
        let ast = must_not_clause(child);
        match ast {
            UserInputAst::Clause(children) => {
                assert_eq!(children.len(), 1);
                assert_eq!(children[0].0, Some(Occur::MustNot));
            }
            _ => panic!("expected Clause"),
        }
    }

    #[test]
    fn field_term_exact_set_produces_set() {
        let ast = field_term_exact_set("format", vec!["epub".to_string(), "pdf".to_string()]);
        match ast {
            UserInputAst::Leaf(leaf) => match *leaf {
                UserInputLeaf::Set { field, elements } => {
                    assert_eq!(field, Some("format".to_string()));
                    assert_eq!(elements, vec!["epub", "pdf"]);
                }
                _ => panic!("expected Set"),
            },
            _ => panic!("expected Leaf"),
        }
    }
}
