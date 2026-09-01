//! A minimal, dependency-free S-expression reader for the SMT-LIB documents Lens
//! emits and the `get-model` bodies z3 prints.
//!
//! Scope is deliberately small: enough to parse `(assert ...)` forms and z3
//! `(define-fun ...)` / `(declare-fun ...)` model entries for the M4 counter-model
//! validator. Atoms are whitespace/paren-delimited tokens (quoted strings are not
//! needed by the emitted documents). Line comments begin with `;` and run to EOL —
//! z3 model bodies carry `;;`-prefixed universe annotations.

/// A parsed S-expression: either an atom (a bare token) or a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sexpr {
    /// A bare token (symbol, numeral, keyword).
    Atom(String),
    /// A parenthesised list of sub-expressions.
    List(Vec<Sexpr>),
}

impl Sexpr {
    /// Render back to a compact S-expression string (used in diagnostic messages).
    pub fn render(&self) -> String {
        match self {
            Sexpr::Atom(a) => a.clone(),
            Sexpr::List(items) => {
                let inner: Vec<String> = items.iter().map(Sexpr::render).collect();
                format!("({})", inner.join(" "))
            }
        }
    }
}

/// Parse every top-level S-expression in `input` (atoms and lists), skipping line
/// comments (`;` … EOL). Returns an error on an unbalanced `)` or an unterminated
/// `(`.
pub fn parse_all(input: &str) -> Result<Vec<Sexpr>, String> {
    let tokens = tokenize(input);
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < tokens.len() {
        let (expr, next) = parse_one(&tokens, pos)?;
        out.push(expr);
        pos = next;
    }
    Ok(out)
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    Atom(String),
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    let mut cur = String::new();
    let flush = |cur: &mut String, tokens: &mut Vec<Token>| {
        if !cur.is_empty() {
            tokens.push(Token::Atom(std::mem::take(cur)));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            ';' => {
                // Line comment to EOL.
                flush(&mut cur, &mut tokens);
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            '(' => {
                flush(&mut cur, &mut tokens);
                tokens.push(Token::Open);
            }
            ')' => {
                flush(&mut cur, &mut tokens);
                tokens.push(Token::Close);
            }
            c if c.is_whitespace() => flush(&mut cur, &mut tokens),
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut tokens);
    tokens
}

fn parse_one(tokens: &[Token], pos: usize) -> Result<(Sexpr, usize), String> {
    match tokens.get(pos) {
        Some(Token::Atom(a)) => Ok((Sexpr::Atom(a.clone()), pos + 1)),
        Some(Token::Open) => {
            let mut items = Vec::new();
            let mut p = pos + 1;
            loop {
                match tokens.get(p) {
                    Some(Token::Close) => return Ok((Sexpr::List(items), p + 1)),
                    None => return Err("unterminated '(' in S-expression".to_string()),
                    Some(_) => {
                        let (expr, next) = parse_one(tokens, p)?;
                        items.push(expr);
                        p = next;
                    }
                }
            }
        }
        Some(Token::Close) => Err("unexpected ')' in S-expression".to_string()),
        None => Err("unexpected end of input".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_list() {
        let s = parse_all("(assert (not (= a b)))").unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].render(), "(assert (not (= a b)))");
    }

    #[test]
    fn skips_line_comments() {
        let s = parse_all(";; universe for PubKey\n(define-fun x () Int 1)").unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].render(), "(define-fun x () Int 1)");
    }

    #[test]
    fn parses_multiple_top_level_forms() {
        let s = parse_all("(declare-const x Int)\n(assert (>= x 0))").unwrap();
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn unbalanced_close_is_error() {
        assert!(parse_all(")").is_err());
    }

    #[test]
    fn unterminated_open_is_error() {
        assert!(parse_all("(assert (>= x 0)").is_err());
    }

    #[test]
    fn negative_literal_list_form() {
        let s = parse_all("(- 5)").unwrap();
        assert_eq!(
            s[0],
            Sexpr::List(vec![Sexpr::Atom("-".into()), Sexpr::Atom("5".into())])
        );
    }
}
