//! Permission expression string parser.
//!
//! Parses CEL-like permission expression strings into `PredicateNode` trees
//! that can be evaluated by `evaluate_predicate`.
//!
//! # Grammar
//!
//! ```text
//! expr        = or_expr
//! or_expr     = and_expr ( "||" and_expr )*
//! and_expr    = unary_expr ( "&&" unary_expr )*
//! unary_expr  = "!" unary_expr | primary
//! primary     = "(" expr ")" | comparison
//! comparison  = field_ref ( cmp_op value_or_ref | "in" field_ref | "contains" value_or_ref
//!             | "containsAll" value_or_ref | "containsAny" value_or_ref
//!             | "startsWith" value_or_ref | "endsWith" value_or_ref )
//! cmp_op      = "==" | "!=" | ">" | "<" | ">=" | "<="
//! field_ref   = IDENT ( "." IDENT )*
//! value_or_ref = field_ref | STRING | NUMBER | BOOLEAN
//! STRING      = "'" [^']* "'"
//! NUMBER      = [0-9]+ ("." [0-9]+)?
//! BOOLEAN     = "true" | "false"
//! IDENT       = [a-zA-Z_][a-zA-Z0-9_]*
//! ```

use topgun_core::messages::base::{PredicateNode, PredicateOp};

// ---------------------------------------------------------------------------
// ParseError
// ---------------------------------------------------------------------------

/// Error returned when parsing a permission expression fails.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    /// Human-readable description of what went wrong.
    pub message: String,
    /// Byte offset in the input where the error was detected.
    pub position: usize,
    /// Up to 20 characters around the error position for context.
    pub snippet: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let caret_offset = self.position.min(self.snippet.len());
        let caret = " ".repeat(caret_offset) + "^";
        write!(
            f,
            "parse error at position {}: {}\n  {}\n  {}",
            self.position, self.message, self.snippet, caret
        )
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Token types
// ---------------------------------------------------------------------------

/// Tokens produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    /// An identifier segment or keyword: `auth`, `data`, `true`, `false`, etc.
    Ident(String),
    /// A dotted field reference like `data.age` or `auth.id`.
    FieldRef(Vec<String>),
    /// A single-quoted string literal (quotes stripped).
    StringLit(String),
    /// An integer literal.
    IntLit(i64),
    /// A floating-point literal.
    FloatLit(f64),
    /// `true`
    True,
    /// `false`
    False,
    /// `null`
    Null,
    /// `==`
    Eq,
    /// `!=`
    Neq,
    /// `>`
    Gt,
    /// `>=`
    Gte,
    /// `<`
    Lt,
    /// `<=`
    Lte,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `!`
    Not,
    /// `in`
    In,
    /// `contains`
    Contains,
    /// `containsAll`
    ContainsAll,
    /// `containsAny`
    ContainsAny,
    /// `startsWith`
    StartsWith,
    /// `endsWith`
    EndsWith,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// End of input.
    Eof,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parses a permission expression string into a `PredicateNode` tree.
///
/// Returns `Err(ParseError)` with position information on malformed input.
///
/// # Errors
///
/// Returns `ParseError` if the input does not conform to the permission expression grammar.
pub fn parse_permission_expr(input: &str) -> Result<PredicateNode, ParseError> {
    let mut parser = Parser::new(input);
    let node = parser.parse_expr()?;
    // Ensure entire input was consumed.
    if parser.current_token() != &Token::Eof {
        let pos = parser.current_pos();
        return Err(parser.error_at(pos, "unexpected token after expression"));
    }
    Ok(node)
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Lexer { input, pos: 0 }
    }

    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Tokenizes the next token from the input.
    #[allow(clippy::too_many_lines)]
    fn next_token(&mut self) -> Result<(usize, Token), ParseError> {
        self.skip_whitespace();
        let start = self.pos;

        let Some(ch) = self.peek_char() else {
            return Ok((self.pos, Token::Eof));
        };

        match ch {
            '(' => {
                self.advance();
                Ok((start, Token::LParen))
            }
            ')' => {
                self.advance();
                Ok((start, Token::RParen))
            }
            '!' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok((start, Token::Neq))
                } else {
                    Ok((start, Token::Not))
                }
            }
            '=' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok((start, Token::Eq))
                } else {
                    Err(self.make_error(start, "expected '==' (single '=' is not valid)"))
                }
            }
            '>' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok((start, Token::Gte))
                } else {
                    Ok((start, Token::Gt))
                }
            }
            '<' => {
                self.advance();
                if self.peek_char() == Some('=') {
                    self.advance();
                    Ok((start, Token::Lte))
                } else {
                    Ok((start, Token::Lt))
                }
            }
            '&' => {
                self.advance();
                if self.peek_char() == Some('&') {
                    self.advance();
                    Ok((start, Token::And))
                } else {
                    Err(self.make_error(start, "expected '&&'"))
                }
            }
            '|' => {
                self.advance();
                if self.peek_char() == Some('|') {
                    self.advance();
                    Ok((start, Token::Or))
                } else {
                    Err(self.make_error(start, "expected '||'"))
                }
            }
            '\'' => {
                self.advance(); // consume opening quote
                let mut s = String::new();
                loop {
                    match self.advance() {
                        None => {
                            return Err(self.make_error(start, "unterminated string literal"));
                        }
                        Some('\'') => break,
                        Some(c) => s.push(c),
                    }
                }
                Ok((start, Token::StringLit(s)))
            }
            c if c.is_ascii_digit() => {
                let num_start = self.pos;
                while self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
                    self.advance();
                }
                let has_dot = self.peek_char() == Some('.');
                // Only treat as float if followed by a digit (not just a lone dot).
                let is_float = has_dot && {
                    let after_dot = self.input.get(self.pos + 1..).and_then(|s| s.chars().next());
                    after_dot.is_some_and(|c| c.is_ascii_digit())
                };
                if is_float {
                    self.advance(); // consume '.'
                    while self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
                        self.advance();
                    }
                    let s = &self.input[num_start..self.pos];
                    let v: f64 = s.parse().map_err(|_| {
                        self.make_error(num_start, "invalid float literal")
                    })?;
                    Ok((start, Token::FloatLit(v)))
                } else {
                    let s = &self.input[num_start..self.pos];
                    let v: i64 = s.parse().map_err(|_| {
                        self.make_error(num_start, "invalid integer literal")
                    })?;
                    Ok((start, Token::IntLit(v)))
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                // Read the first identifier segment.
                let seg_start = self.pos;
                while self.peek_char().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                    self.advance();
                }
                let first = self.input[seg_start..self.pos].to_string();

                // Check for operator keywords before dot-path collection.
                // The full identifier word is read before matching, so longer keywords
                // like "containsAll" cannot be confused with "contains".
                match first.as_str() {
                    "in" => return Ok((start, Token::In)),
                    "containsAll" => return Ok((start, Token::ContainsAll)),
                    "containsAny" => return Ok((start, Token::ContainsAny)),
                    "contains" => return Ok((start, Token::Contains)),
                    "startsWith" => return Ok((start, Token::StartsWith)),
                    "endsWith" => return Ok((start, Token::EndsWith)),
                    _ => {}
                }

                // Check if followed by a dot — if so, collect a dotted field_ref.
                if self.peek_char() == Some('.') {
                    let mut segments = vec![first];
                    while self.peek_char() == Some('.') {
                        self.advance(); // consume '.'
                        let seg_s = self.pos;
                        if !self.peek_char().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
                            return Err(self.make_error(seg_s, "expected identifier after '.'"));
                        }
                        while self.peek_char().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                            self.advance();
                        }
                        segments.push(self.input[seg_s..self.pos].to_string());
                    }
                    return Ok((start, Token::FieldRef(segments)));
                }

                // Single identifier — check for keywords.
                let tok = match first.as_str() {
                    "true" => Token::True,
                    "false" => Token::False,
                    "null" => Token::Null,
                    _ => Token::Ident(first),
                };
                Ok((start, tok))
            }
            other => {
                self.advance();
                Err(self.make_error(start, &format!("unexpected character '{other}'")))
            }
        }
    }

    fn make_error(&self, pos: usize, message: &str) -> ParseError {
        let snippet = make_snippet(self.input, pos);
        ParseError {
            message: message.to_string(),
            position: pos,
            snippet,
        }
    }
}

/// Extracts up to 20 characters of context around `pos` from `input`.
fn make_snippet(input: &str, pos: usize) -> String {
    let start = pos.saturating_sub(5);
    let end = (pos + 15).min(input.len());
    input[start..end].to_string()
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    /// Current token and its position.
    current: (usize, Token),
    /// Whether the last lex attempt produced an error (stored for re-reporting).
    lex_error: Option<ParseError>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        let mut lexer = Lexer::new(input);
        let (current, lex_error) = match lexer.next_token() {
            Ok(tok) => (tok, None),
            Err(e) => {
                let pos = e.position;
                ((pos, Token::Eof), Some(e))
            }
        };
        Parser {
            input,
            lexer,
            current,
            lex_error,
        }
    }

    fn current_token(&self) -> &Token {
        &self.current.1
    }

    fn current_pos(&self) -> usize {
        self.current.0
    }

    /// Advance to the next token, returning the consumed token's position.
    fn advance(&mut self) -> Result<(usize, Token), ParseError> {
        if let Some(e) = self.lex_error.take() {
            return Err(e);
        }
        let consumed = std::mem::replace(&mut self.current, (0, Token::Eof));
        self.current = match self.lexer.next_token() {
            Ok(tok) => tok,
            Err(e) => {
                let pos = e.position;
                self.lex_error = Some(e);
                (pos, Token::Eof)
            }
        };
        Ok(consumed)
    }

    fn error_at(&self, pos: usize, message: &str) -> ParseError {
        ParseError {
            message: message.to_string(),
            position: pos,
            snippet: make_snippet(self.input, pos),
        }
    }

    // ---- Grammar productions ----

    // expr = or_expr
    fn parse_expr(&mut self) -> Result<PredicateNode, ParseError> {
        self.parse_or_expr()
    }

    // or_expr = and_expr ( "||" and_expr )*
    fn parse_or_expr(&mut self) -> Result<PredicateNode, ParseError> {
        let mut left = self.parse_and_expr()?;

        while self.current_token() == &Token::Or {
            self.advance()?;
            let right = self.parse_and_expr()?;
            // Flatten: if left is already an Or combinator, add right as another child.
            if left.op == PredicateOp::Or {
                left.children.as_mut().unwrap().push(right);
            } else {
                left = PredicateNode {
                    op: PredicateOp::Or,
                    attribute: None,
                    value: None,
                    value_ref: None,
                    children: Some(vec![left, right]),
                };
            }
        }

        Ok(left)
    }

    // and_expr = unary_expr ( "&&" unary_expr )*
    fn parse_and_expr(&mut self) -> Result<PredicateNode, ParseError> {
        let mut left = self.parse_unary_expr()?;

        while self.current_token() == &Token::And {
            self.advance()?;
            let right = self.parse_unary_expr()?;
            // Flatten: if left is already an And combinator, add right as another child.
            if left.op == PredicateOp::And {
                left.children.as_mut().unwrap().push(right);
            } else {
                left = PredicateNode {
                    op: PredicateOp::And,
                    attribute: None,
                    value: None,
                    value_ref: None,
                    children: Some(vec![left, right]),
                };
            }
        }

        Ok(left)
    }

    // unary_expr = "!" unary_expr | primary
    fn parse_unary_expr(&mut self) -> Result<PredicateNode, ParseError> {
        if self.current_token() == &Token::Not {
            self.advance()?;
            let operand = self.parse_unary_expr()?;
            return Ok(PredicateNode {
                op: PredicateOp::Not,
                attribute: None,
                value: None,
                value_ref: None,
                children: Some(vec![operand]),
            });
        }
        self.parse_primary()
    }

    // primary = "(" expr ")" | comparison
    fn parse_primary(&mut self) -> Result<PredicateNode, ParseError> {
        if self.current_token() == &Token::LParen {
            self.advance()?; // consume '('
            let node = self.parse_expr()?;
            if self.current_token() != &Token::RParen {
                let pos = self.current_pos();
                return Err(self.error_at(pos, "expected ')'"));
            }
            self.advance()?; // consume ')'
            return Ok(node);
        }
        self.parse_comparison()
    }

    // comparison = field_ref ( cmp_op value_or_ref | "in" field_ref )
    fn parse_comparison(&mut self) -> Result<PredicateNode, ParseError> {
        let (lhs_pos, lhs_segments) = self.parse_field_ref_raw()?;

        // Require a comparison operator, "in", or "contains".
        match self.current_token().clone() {
            Token::In => {
                self.advance()?;
                let (_rhs_pos, rhs_segments) = self.parse_field_ref_raw()?;
                build_in_node(&lhs_segments, &rhs_segments, lhs_pos, self.input)
            }
            Token::Contains => {
                self.advance()?;
                let rhs = self.parse_value_or_ref(lhs_pos)?;
                build_contains_node(&lhs_segments, rhs, lhs_pos, self.input)
            }
            Token::ContainsAll => {
                self.advance()?;
                let rhs = self.parse_value_or_ref(lhs_pos)?;
                build_data_lhs_node(PredicateOp::ContainsAll, &lhs_segments, rhs, lhs_pos, self.input)
            }
            Token::ContainsAny => {
                self.advance()?;
                let rhs = self.parse_value_or_ref(lhs_pos)?;
                build_data_lhs_node(PredicateOp::ContainsAny, &lhs_segments, rhs, lhs_pos, self.input)
            }
            Token::StartsWith => {
                self.advance()?;
                let rhs = self.parse_value_or_ref(lhs_pos)?;
                build_data_lhs_node(PredicateOp::StartsWith, &lhs_segments, rhs, lhs_pos, self.input)
            }
            Token::EndsWith => {
                self.advance()?;
                let rhs = self.parse_value_or_ref(lhs_pos)?;
                build_data_lhs_node(PredicateOp::EndsWith, &lhs_segments, rhs, lhs_pos, self.input)
            }
            Token::Eq
            | Token::Neq
            | Token::Gt
            | Token::Gte
            | Token::Lt
            | Token::Lte => {
                let op_tok = self.current_token().clone();
                let op_pos = self.current_pos();
                self.advance()?;
                let rhs = self.parse_value_or_ref(op_pos)?;
                Ok(build_comparison_node(&lhs_segments, &op_tok, rhs))
            }
            _ => {
                let pos = self.current_pos();
                // A bare field_ref without a comparison operator is a parse error.
                Err(self.error_at(
                    pos,
                    "expected comparison operator (==, !=, >, >=, <, <=), 'in', 'contains', 'containsAll', 'containsAny', 'startsWith', or 'endsWith'",
                ))
            }
        }
    }

    // Parses a field_ref (dotted identifier), returning (position, segments).
    fn parse_field_ref_raw(&mut self) -> Result<(usize, Vec<String>), ParseError> {
        let pos = self.current_pos();
        match self.current_token().clone() {
            Token::FieldRef(segs) => {
                self.advance()?;
                Ok((pos, segs))
            }
            Token::Ident(name) => {
                self.advance()?;
                Ok((pos, vec![name]))
            }
            // true/false are valid idents in field position (edge case; treated as bare name)
            Token::True => {
                self.advance()?;
                Ok((pos, vec!["true".to_string()]))
            }
            Token::False => {
                self.advance()?;
                Ok((pos, vec!["false".to_string()]))
            }
            // null is not a valid left-hand side in a comparison
            Token::Null => {
                Err(self.error_at(pos, "null literal is not valid as left-hand side of a comparison"))
            }
            _ => {
                Err(self.error_at(pos, "expected field reference (e.g. data.age or auth.id)"))
            }
        }
    }

    // value_or_ref = field_ref | STRING | NUMBER | BOOLEAN
    fn parse_value_or_ref(&mut self, _op_pos: usize) -> Result<RhsValue, ParseError> {
        let pos = self.current_pos();
        match self.current_token().clone() {
            Token::FieldRef(segs) => {
                self.advance()?;
                Ok(RhsValue::FieldRef(segs))
            }
            Token::Ident(name) => {
                self.advance()?;
                Ok(RhsValue::FieldRef(vec![name]))
            }
            Token::StringLit(s) => {
                self.advance()?;
                Ok(RhsValue::Str(s))
            }
            Token::IntLit(n) => {
                self.advance()?;
                Ok(RhsValue::Int(n))
            }
            Token::FloatLit(f) => {
                self.advance()?;
                Ok(RhsValue::Float(f))
            }
            Token::True => {
                self.advance()?;
                Ok(RhsValue::Bool(true))
            }
            Token::False => {
                self.advance()?;
                Ok(RhsValue::Bool(false))
            }
            Token::Null => {
                self.advance()?;
                Ok(RhsValue::Null)
            }
            _ => Err(self.error_at(pos, "expected value or field reference after operator")),
        }
    }
}

// ---------------------------------------------------------------------------
// RHS value helper
// ---------------------------------------------------------------------------

enum RhsValue {
    FieldRef(Vec<String>),
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
}

// ---------------------------------------------------------------------------
// Node construction helpers
// ---------------------------------------------------------------------------

/// Builds a comparison `PredicateNode` from parsed LHS segments, operator token, and RHS value.
///
/// Applies field reference resolution rules. `data.X op auth.Y` maps to
/// `attribute = "X"` with `value_ref = "auth.Y"`. When auth is on the left and
/// data on the right, sides are swapped automatically.
#[allow(clippy::too_many_lines)]
fn build_comparison_node(lhs: &[String], op_tok: &Token, rhs: RhsValue) -> PredicateNode {
    let op = match op_tok {
        Token::Eq => PredicateOp::Eq,
        Token::Neq => PredicateOp::Neq,
        Token::Gt => PredicateOp::Gt,
        Token::Gte => PredicateOp::Gte,
        Token::Lt => PredicateOp::Lt,
        Token::Lte => PredicateOp::Lte,
        _ => unreachable!("build_comparison_node called with non-cmp token"),
    };

    // Determine if LHS is a data or auth reference.
    let lhs_is_data = lhs.first().is_some_and(|s| s == "data");
    let lhs_is_auth = lhs.first().is_some_and(|s| s == "auth");

    match rhs {
        RhsValue::FieldRef(rhs_segs) => {
            let rhs_is_data = rhs_segs.first().is_some_and(|s| s == "data");
            let rhs_is_auth = rhs_segs.first().is_some_and(|s| s == "auth");

            if lhs_is_data && rhs_is_auth {
                // Normal case: data.X op auth.Y -> attribute=X, value_ref="auth.Y"
                let attribute = lhs[1..].join(".");
                let value_ref = rhs_segs.join(".");
                PredicateNode {
                    op,
                    attribute: Some(attribute),
                    value: None,
                    value_ref: Some(value_ref),
                    children: None,
                }
            } else if lhs_is_auth && rhs_is_data {
                // Swap: auth.X op data.Y -> attribute=Y, value_ref="auth.X"
                let attribute = rhs_segs[1..].join(".");
                let value_ref = lhs.join(".");
                PredicateNode {
                    op,
                    attribute: Some(attribute),
                    value: None,
                    value_ref: Some(value_ref),
                    children: None,
                }
            } else if lhs_is_data {
                // data.X op other.Y -> attribute=X, value_ref="other.Y"
                let attribute = lhs[1..].join(".");
                let value_ref = rhs_segs.join(".");
                PredicateNode {
                    op,
                    attribute: Some(attribute),
                    value: None,
                    value_ref: Some(value_ref),
                    children: None,
                }
            } else {
                // Fallback: use full paths.
                let attribute = lhs.join(".");
                let value_ref = rhs_segs.join(".");
                PredicateNode {
                    op,
                    attribute: Some(attribute),
                    value: None,
                    value_ref: Some(value_ref),
                    children: None,
                }
            }
        }
        RhsValue::Str(s) => {
            let attribute = if lhs_is_data {
                lhs[1..].join(".")
            } else {
                lhs.join(".")
            };
            PredicateNode {
                op,
                attribute: Some(attribute),
                value: Some(rmpv::Value::String(s.into())),
                value_ref: None,
                children: None,
            }
        }
        RhsValue::Int(n) => {
            let attribute = if lhs_is_data {
                lhs[1..].join(".")
            } else {
                lhs.join(".")
            };
            PredicateNode {
                op,
                attribute: Some(attribute),
                value: Some(rmpv::Value::Integer(n.into())),
                value_ref: None,
                children: None,
            }
        }
        RhsValue::Float(f) => {
            let attribute = if lhs_is_data {
                lhs[1..].join(".")
            } else {
                lhs.join(".")
            };
            PredicateNode {
                op,
                attribute: Some(attribute),
                value: Some(rmpv::Value::F64(f)),
                value_ref: None,
                children: None,
            }
        }
        RhsValue::Bool(b) => {
            let attribute = if lhs_is_data {
                lhs[1..].join(".")
            } else {
                lhs.join(".")
            };
            PredicateNode {
                op,
                attribute: Some(attribute),
                value: Some(rmpv::Value::Boolean(b)),
                value_ref: None,
                children: None,
            }
        }
        RhsValue::Null => {
            // For equality/inequality with null, emit IsNull/IsNotNull.
            // Strip the `data.` prefix but keep the full `auth` or `auth.X` path.
            let attribute = if lhs_is_data {
                lhs[1..].join(".")
            } else {
                lhs.join(".")
            };
            match op_tok {
                Token::Eq => PredicateNode {
                    op: PredicateOp::IsNull,
                    attribute: Some(attribute),
                    value: None,
                    value_ref: None,
                    children: None,
                },
                Token::Neq => PredicateNode {
                    op: PredicateOp::IsNotNull,
                    attribute: Some(attribute),
                    value: None,
                    value_ref: None,
                    children: None,
                },
                // Ordering comparisons against null are meaningless; produce an always-false leaf.
                _ => PredicateNode {
                    op: PredicateOp::Eq,
                    attribute: None,
                    value: None,
                    value_ref: None,
                    children: None,
                },
            }
        }
    }
}

/// Builds an `In` node: `data.role in auth.roles` -> `{ op: In, attribute: "role", value_ref: "auth.roles" }`.
fn build_in_node(
    lhs: &[String],
    rhs: &[String],
    lhs_pos: usize,
    input: &str,
) -> Result<PredicateNode, ParseError> {
    let lhs_is_data = lhs.first().is_some_and(|s| s == "data");
    let rhs_is_auth = rhs.first().is_some_and(|s| s == "auth");

    if lhs_is_data && rhs_is_auth {
        let attribute = lhs[1..].join(".");
        let value_ref = rhs.join(".");
        Ok(PredicateNode {
            op: PredicateOp::In,
            attribute: Some(attribute),
            value: None,
            value_ref: Some(value_ref),
            children: None,
        })
    } else if lhs_is_data {
        // data.X in some.ref -> attribute=X, value_ref=ref
        let attribute = lhs[1..].join(".");
        let value_ref = rhs.join(".");
        Ok(PredicateNode {
            op: PredicateOp::In,
            attribute: Some(attribute),
            value: None,
            value_ref: Some(value_ref),
            children: None,
        })
    } else {
        Err(ParseError {
            message: "left side of 'in' must be a data.* field reference".to_string(),
            position: lhs_pos,
            snippet: make_snippet(input, lhs_pos),
        })
    }
}

/// Builds an `In` node for `data.array contains scalar` semantics.
///
/// The node uses `PredicateOp::In` with swapped positions so the existing
/// bidirectional `evaluate_in` can detect the array-contains-scalar case:
/// - `attribute` is the LHS data field (the array field)
/// - `value` is the scalar literal RHS, or `value_ref` for a field-reference RHS
///
/// The LHS must start with `data.`; non-data references return a `ParseError`.
fn build_contains_node(
    lhs: &[String],
    rhs: RhsValue,
    lhs_pos: usize,
    input: &str,
) -> Result<PredicateNode, ParseError> {
    let lhs_is_data = lhs.first().is_some_and(|s| s == "data");

    if !lhs_is_data {
        return Err(ParseError {
            message: "left side of 'contains' must be a data.* field reference".to_string(),
            position: lhs_pos,
            snippet: make_snippet(input, lhs_pos),
        });
    }

    let attribute = lhs[1..].join(".");

    let node = match rhs {
        RhsValue::FieldRef(segs) => PredicateNode {
            op: PredicateOp::In,
            attribute: Some(attribute),
            value: None,
            value_ref: Some(segs.join(".")),
            children: None,
        },
        RhsValue::Str(s) => PredicateNode {
            op: PredicateOp::In,
            attribute: Some(attribute),
            value: Some(rmpv::Value::String(s.into())),
            value_ref: None,
            children: None,
        },
        RhsValue::Int(n) => PredicateNode {
            op: PredicateOp::In,
            attribute: Some(attribute),
            value: Some(rmpv::Value::Integer(n.into())),
            value_ref: None,
            children: None,
        },
        RhsValue::Float(f) => PredicateNode {
            op: PredicateOp::In,
            attribute: Some(attribute),
            value: Some(rmpv::Value::F64(f)),
            value_ref: None,
            children: None,
        },
        RhsValue::Bool(b) => PredicateNode {
            op: PredicateOp::In,
            attribute: Some(attribute),
            value: Some(rmpv::Value::Boolean(b)),
            value_ref: None,
            children: None,
        },
        // null on the right side of `contains` is not meaningful; produce an always-false leaf.
        RhsValue::Null => PredicateNode {
            op: PredicateOp::Eq,
            attribute: None,
            value: None,
            value_ref: None,
            children: None,
        },
    };

    Ok(node)
}

/// Builds a `PredicateNode` for operators that require `data.*` on the left-hand side.
///
/// Used by `containsAll`, `containsAny`, `startsWith`, and `endsWith`. Validates that
/// the LHS is a `data.*` field reference and strips the `data.` prefix from `attribute`.
/// Returns a `ParseError` if the LHS does not start with `data.`.
fn build_data_lhs_node(
    op: PredicateOp,
    lhs: &[String],
    rhs: RhsValue,
    lhs_pos: usize,
    input: &str,
) -> Result<PredicateNode, ParseError> {
    let lhs_is_data = lhs.first().is_some_and(|s| s == "data");

    if !lhs_is_data {
        return Err(ParseError {
            message: format!(
                "left side of '{}' must be a data.* field reference",
                match op {
                    PredicateOp::ContainsAll => "containsAll",
                    PredicateOp::ContainsAny => "containsAny",
                    PredicateOp::StartsWith => "startsWith",
                    PredicateOp::EndsWith => "endsWith",
                    _ => "operator",
                }
            ),
            position: lhs_pos,
            snippet: make_snippet(input, lhs_pos),
        });
    }

    let attribute = lhs[1..].join(".");

    let node = match rhs {
        RhsValue::FieldRef(segs) => PredicateNode {
            op,
            attribute: Some(attribute),
            value: None,
            value_ref: Some(segs.join(".")),
            children: None,
        },
        RhsValue::Str(s) => PredicateNode {
            op,
            attribute: Some(attribute),
            value: Some(rmpv::Value::String(s.into())),
            value_ref: None,
            children: None,
        },
        RhsValue::Int(n) => PredicateNode {
            op,
            attribute: Some(attribute),
            value: Some(rmpv::Value::Integer(n.into())),
            value_ref: None,
            children: None,
        },
        RhsValue::Float(f) => PredicateNode {
            op,
            attribute: Some(attribute),
            value: Some(rmpv::Value::F64(f)),
            value_ref: None,
            children: None,
        },
        RhsValue::Bool(b) => PredicateNode {
            op,
            attribute: Some(attribute),
            value: Some(rmpv::Value::Boolean(b)),
            value_ref: None,
            children: None,
        },
        // null on the right side is not meaningful; produce an always-false leaf.
        RhsValue::Null => PredicateNode {
            op: PredicateOp::Eq,
            attribute: None,
            value: None,
            value_ref: None,
            children: None,
        },
    };

    Ok(node)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::messages::base::PredicateOp;

    // Helper: assert parse succeeds and matches expected op/attribute/value_ref.
    fn parsed(input: &str) -> PredicateNode {
        parse_permission_expr(input).unwrap_or_else(|e| panic!("parse failed for '{input}': {e}"))
    }

    fn parse_err(input: &str) -> ParseError {
        parse_permission_expr(input)
            .err()
            .unwrap_or_else(|| panic!("expected parse error for '{input}' but parse succeeded"))
    }

    // --- AC1: auth.id == data.ownerId ---

    #[test]
    fn ac1_auth_eq_data() {
        let node = parsed("auth.id == data.ownerId");
        assert_eq!(node.op, PredicateOp::Eq);
        assert_eq!(node.attribute, Some("ownerId".to_string()));
        assert_eq!(node.value_ref, Some("auth.id".to_string()));
        assert!(node.value.is_none());
        assert!(node.children.is_none());
    }

    // --- AC2: data.age >= 18 && data.status == 'active' ---

    #[test]
    fn ac2_and_with_int_and_string() {
        let node = parsed("data.age >= 18 && data.status == 'active'");
        assert_eq!(node.op, PredicateOp::And);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);

        let age_node = &children[0];
        assert_eq!(age_node.op, PredicateOp::Gte);
        assert_eq!(age_node.attribute, Some("age".to_string()));
        assert_eq!(age_node.value, Some(rmpv::Value::Integer(18.into())));

        let status_node = &children[1];
        assert_eq!(status_node.op, PredicateOp::Eq);
        assert_eq!(status_node.attribute, Some("status".to_string()));
        assert_eq!(
            status_node.value,
            Some(rmpv::Value::String("active".into()))
        );
    }

    // --- AC3: data.role in auth.roles || data.public == true ---

    #[test]
    fn ac3_in_or_boolean() {
        let node = parsed("data.role in auth.roles || data.public == true");
        assert_eq!(node.op, PredicateOp::Or);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);

        let in_node = &children[0];
        assert_eq!(in_node.op, PredicateOp::In);
        assert_eq!(in_node.attribute, Some("role".to_string()));
        assert_eq!(in_node.value_ref, Some("auth.roles".to_string()));

        let public_node = &children[1];
        assert_eq!(public_node.op, PredicateOp::Eq);
        assert_eq!(public_node.attribute, Some("public".to_string()));
        assert_eq!(public_node.value, Some(rmpv::Value::Boolean(true)));
    }

    // --- AC4: !data.deleted returns error (bare field ref, no comparison) ---

    #[test]
    fn ac4_not_with_bare_field_ref_is_error() {
        let err = parse_err("!data.deleted");
        // The inner unary_expr calls parse_primary -> parse_comparison -> parse_field_ref_raw
        // which succeeds, then sees Eof instead of a comparison operator.
        assert!(
            err.message.contains("expected comparison operator"),
            "unexpected error: {err}"
        );
    }

    // --- AC5: data.x == returns error with position at end ---

    #[test]
    fn ac5_missing_rhs_gives_error_at_eof() {
        let err = parse_err("data.x ==");
        assert!(
            err.message
                .contains("expected value or field reference after operator"),
            "unexpected error: {err}"
        );
        // Position should be at or near end of input (length 9).
        assert!(err.position >= 9, "expected position near end, got {}", err.position);
    }

    // --- AC6: nested parenthesized Or inside And ---

    #[test]
    fn ac6_nested_parens() {
        let node = parsed("data.x == 'hello' && (auth.role == 'admin' || auth.role == 'editor')");
        assert_eq!(node.op, PredicateOp::And);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);

        // First child: data.x == 'hello'
        assert_eq!(children[0].op, PredicateOp::Eq);
        assert_eq!(children[0].attribute, Some("x".to_string()));

        // Second child: Or node
        let or_node = &children[1];
        assert_eq!(or_node.op, PredicateOp::Or);
        let or_children = or_node.children.as_ref().unwrap();
        assert_eq!(or_children.len(), 2);
        assert_eq!(or_children[0].op, PredicateOp::Eq);
        assert_eq!(or_children[1].op, PredicateOp::Eq);
    }

    // --- All comparison operators ---

    #[test]
    fn all_cmp_operators() {
        let cases = [
            ("data.age == 18", PredicateOp::Eq),
            ("data.age != 18", PredicateOp::Neq),
            ("data.age > 18", PredicateOp::Gt),
            ("data.age >= 18", PredicateOp::Gte),
            ("data.age < 18", PredicateOp::Lt),
            ("data.age <= 18", PredicateOp::Lte),
        ];
        for (expr, expected_op) in &cases {
            let node = parsed(expr);
            assert_eq!(&node.op, expected_op, "failed for: {expr}");
            assert_eq!(node.attribute, Some("age".to_string()));
            assert_eq!(node.value, Some(rmpv::Value::Integer(18.into())));
        }
    }

    // --- Not combinator ---

    #[test]
    fn not_combinator() {
        let node = parsed("!(data.age == 18)");
        assert_eq!(node.op, PredicateOp::Not);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].op, PredicateOp::Eq);
    }

    // --- Float literal ---

    #[test]
    fn float_literal() {
        let node = parsed("data.score >= 9.5");
        assert_eq!(node.op, PredicateOp::Gte);
        assert_eq!(node.attribute, Some("score".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::F64(9.5)));
    }

    // --- Boolean false ---

    #[test]
    fn boolean_false_literal() {
        let node = parsed("data.deleted == false");
        assert_eq!(node.op, PredicateOp::Eq);
        assert_eq!(node.attribute, Some("deleted".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::Boolean(false)));
    }

    // --- data.X == data.Y (data-to-data comparison via value_ref) ---

    #[test]
    fn data_to_data_comparison() {
        let node = parsed("data.createdBy == data.ownerId");
        assert_eq!(node.op, PredicateOp::Eq);
        assert_eq!(node.attribute, Some("createdBy".to_string()));
        assert_eq!(node.value_ref, Some("data.ownerId".to_string()));
    }

    // --- Swap: auth.id == data.ownerId (auth on left) ---

    #[test]
    fn swap_auth_lhs_data_rhs() {
        // auth.id == data.ownerId should swap to attribute="ownerId", value_ref="auth.id"
        let node = parsed("auth.id == data.ownerId");
        // Same as AC1 but confirm swap logic
        assert_eq!(node.op, PredicateOp::Eq);
        assert_eq!(node.attribute, Some("ownerId".to_string()));
        assert_eq!(node.value_ref, Some("auth.id".to_string()));
    }

    // --- Multi-segment And (3 terms) ---

    #[test]
    fn three_term_and() {
        let node = parsed("data.a == 1 && data.b == 2 && data.c == 3");
        assert_eq!(node.op, PredicateOp::And);
        // Flattened: 3 children under single And.
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 3);
    }

    // --- Multi-segment Or (3 terms) ---

    #[test]
    fn three_term_or() {
        let node = parsed("data.x == 1 || data.y == 2 || data.z == 3");
        assert_eq!(node.op, PredicateOp::Or);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 3);
    }

    // --- Error: bogus >>> ---

    #[test]
    fn error_bogus_characters() {
        let err = parse_err("bogus >>>");
        // Should have a position and a snippet.
        assert!(!err.snippet.is_empty());
        assert!(err.position < 20);
    }

    // --- Error: data.x >> (double > not supported as token) ---

    #[test]
    fn error_data_x_double_gt() {
        let err = parse_err("data.x >>");
        assert!(err.position > 0);
    }

    // --- Error: unclosed paren ---

    #[test]
    fn error_unclosed_paren() {
        let err = parse_err("(data.x == 1");
        assert!(
            err.message.contains("expected ')'"),
            "unexpected error: {err}"
        );
    }

    // --- Error: trailing token ---

    #[test]
    fn error_trailing_token() {
        let err = parse_err("data.x == 1 extra");
        assert!(
            err.message.contains("unexpected token"),
            "unexpected error: {err}"
        );
    }

    // --- Error: single '=' ---

    #[test]
    fn error_single_equals() {
        let err = parse_err("data.x = 1");
        assert!(err.message.contains("=="), "unexpected error: {err}");
    }

    // --- Error: empty string ---

    #[test]
    fn error_empty_input() {
        let err = parse_err("");
        assert!(
            err.message.contains("expected field reference"),
            "unexpected error: {err}"
        );
    }

    // --- ParseError Display format ---

    #[test]
    fn parse_error_display_includes_position() {
        let err = parse_err("data.x ==");
        let display = err.to_string();
        assert!(display.contains("parse error at position"), "display: {display}");
        assert!(display.contains('^'), "caret missing in display: {display}");
    }

    // --- String with spaces ---

    #[test]
    fn string_with_spaces() {
        let node = parsed("data.name == 'hello world'");
        assert_eq!(
            node.value,
            Some(rmpv::Value::String("hello world".into()))
        );
    }

    // --- Whitespace tolerance ---

    #[test]
    fn whitespace_tolerance() {
        let node = parsed("  data.age  ==  42  ");
        assert_eq!(node.op, PredicateOp::Eq);
        assert_eq!(node.attribute, Some("age".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::Integer(42.into())));
    }

    // -----------------------------------------------------------------------
    // Round-trip tests (G4): parse expression, evaluate with evaluate_predicate
    // -----------------------------------------------------------------------

    use crate::service::domain::predicate::{evaluate_predicate, EvalContext};

    fn make_data(pairs: &[(&str, rmpv::Value)]) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .iter()
                .map(|(k, v)| (rmpv::Value::String((*k).into()), v.clone()))
                .collect(),
        )
    }

    fn make_auth(id: &str, roles: &[&str]) -> rmpv::Value {
        rmpv::Value::Map(vec![
            (
                rmpv::Value::String("id".into()),
                rmpv::Value::String(id.into()),
            ),
            (
                rmpv::Value::String("roles".into()),
                rmpv::Value::Array(
                    roles
                        .iter()
                        .map(|r| rmpv::Value::String((*r).into()))
                        .collect(),
                ),
            ),
        ])
    }

    #[test]
    fn roundtrip_auth_id_eq_owner_id_true() {
        let node = parsed("auth.id == data.ownerId");
        let data = make_data(&[("ownerId", rmpv::Value::String("u1".into()))]);
        let auth = make_auth("u1", &[]);
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_auth_id_eq_owner_id_false() {
        let node = parsed("auth.id == data.ownerId");
        let data = make_data(&[("ownerId", rmpv::Value::String("u2".into()))]);
        let auth = make_auth("u1", &[]);
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(!evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_and_age_status_true() {
        let node = parsed("data.age >= 18 && data.status == 'active'");
        let data = make_data(&[
            ("age", rmpv::Value::Integer(25.into())),
            ("status", rmpv::Value::String("active".into())),
        ]);
        assert!(evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    #[test]
    fn roundtrip_and_age_status_false_age() {
        let node = parsed("data.age >= 18 && data.status == 'active'");
        let data = make_data(&[
            ("age", rmpv::Value::Integer(15.into())),
            ("status", rmpv::Value::String("active".into())),
        ]);
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    #[test]
    fn roundtrip_in_role_present() {
        let node = parsed("data.role in auth.roles");
        let data = make_data(&[("role", rmpv::Value::String("editor".into()))]);
        let auth = make_auth("u1", &["viewer", "editor"]);
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_in_role_absent() {
        let node = parsed("data.role in auth.roles");
        let data = make_data(&[("role", rmpv::Value::String("admin".into()))]);
        let auth = make_auth("u1", &["viewer", "editor"]);
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(!evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_or_in_and_bool_true_via_in() {
        let node = parsed("data.role in auth.roles || data.public == true");
        let data = make_data(&[
            ("role", rmpv::Value::String("editor".into())),
            ("public", rmpv::Value::Boolean(false)),
        ]);
        let auth = make_auth("u1", &["editor"]);
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_or_in_and_bool_true_via_bool() {
        let node = parsed("data.role in auth.roles || data.public == true");
        let data = make_data(&[
            ("role", rmpv::Value::String("guest".into())),
            ("public", rmpv::Value::Boolean(true)),
        ]);
        let auth = make_auth("u1", &["viewer"]);
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_not() {
        let node = parsed("!(data.age == 18)");
        let data_18 = make_data(&[("age", rmpv::Value::Integer(18.into()))]);
        let data_25 = make_data(&[("age", rmpv::Value::Integer(25.into()))]);
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data_18)));
        assert!(evaluate_predicate(&node, &EvalContext::data_only(&data_25)));
    }

    #[test]
    fn roundtrip_nested_parens() {
        let node = parsed(
            "data.x == 'hello' && (data.status == 'admin' || data.status == 'editor')",
        );
        let data_match = make_data(&[
            ("x", rmpv::Value::String("hello".into())),
            ("status", rmpv::Value::String("admin".into())),
        ]);
        let data_no_match = make_data(&[
            ("x", rmpv::Value::String("hello".into())),
            ("status", rmpv::Value::String("viewer".into())),
        ]);
        assert!(evaluate_predicate(&node, &EvalContext::data_only(&data_match)));
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data_no_match)));
    }

    // --- contains keyword parser tests ---

    #[test]
    fn contains_string_literal() {
        // data.tags contains 'vip' -> op: In, attribute: "tags", value: String("vip")
        let node = parsed("data.tags contains 'vip'");
        assert_eq!(node.op, PredicateOp::In);
        assert_eq!(node.attribute, Some("tags".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::String("vip".into())));
        assert!(node.value_ref.is_none());
    }

    #[test]
    fn contains_integer_literal() {
        // data.tags contains 42 -> op: In, attribute: "tags", value: Integer(42)
        let node = parsed("data.tags contains 42");
        assert_eq!(node.op, PredicateOp::In);
        assert_eq!(node.attribute, Some("tags".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::Integer(42.into())));
        assert!(node.value_ref.is_none());
    }

    #[test]
    fn contains_field_reference() {
        // data.tags contains auth.role -> op: In, attribute: "tags", value_ref: "auth.role"
        let node = parsed("data.tags contains auth.role");
        assert_eq!(node.op, PredicateOp::In);
        assert_eq!(node.attribute, Some("tags".to_string()));
        assert_eq!(node.value_ref, Some("auth.role".to_string()));
        assert!(node.value.is_none());
    }

    #[test]
    fn contains_in_compound_and_expression() {
        // data.tags contains 'vip' && data.active == true
        let node = parsed("data.tags contains 'vip' && data.active == true");
        assert_eq!(node.op, PredicateOp::And);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);

        let contains_node = &children[0];
        assert_eq!(contains_node.op, PredicateOp::In);
        assert_eq!(contains_node.attribute, Some("tags".to_string()));
        assert_eq!(contains_node.value, Some(rmpv::Value::String("vip".into())));

        let active_node = &children[1];
        assert_eq!(active_node.op, PredicateOp::Eq);
        assert_eq!(active_node.attribute, Some("active".to_string()));
        assert_eq!(active_node.value, Some(rmpv::Value::Boolean(true)));
    }

    #[test]
    fn contains_lhs_non_data_is_error() {
        // auth.roles contains 'x' -> parse error: LHS must be data.*
        let err = parse_err("auth.roles contains 'x'");
        assert!(
            err.message.contains("left side of 'contains' must be a data.*"),
            "unexpected error: {err}"
        );
    }

    // --- contains round-trip tests ---

    #[test]
    fn roundtrip_contains_true() {
        let node = parsed("data.tags contains 'vip'");
        let data = make_data(&[(
            "tags",
            rmpv::Value::Array(vec![
                rmpv::Value::String("vip".into()),
                rmpv::Value::String("premium".into()),
            ]),
        )]);
        assert!(evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    #[test]
    fn roundtrip_contains_false() {
        let node = parsed("data.tags contains 'vip'");
        let data = make_data(&[(
            "tags",
            rmpv::Value::Array(vec![rmpv::Value::String("basic".into())]),
        )]);
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    #[test]
    fn roundtrip_nested_parens_auth() {
        let node = parsed(
            "(auth.id == data.ownerId || data.public == true) && data.active == true",
        );

        // Owner with inactive record — false because active == false.
        let data_inactive = make_data(&[
            ("ownerId", rmpv::Value::String("u1".into())),
            ("public", rmpv::Value::Boolean(false)),
            ("active", rmpv::Value::Boolean(false)),
        ]);
        let auth = make_auth("u1", &[]);
        assert!(!evaluate_predicate(
            &node,
            &EvalContext { auth: Some(&auth), data: &data_inactive }
        ));

        // Owner with active record — true.
        let data_active_owner = make_data(&[
            ("ownerId", rmpv::Value::String("u1".into())),
            ("public", rmpv::Value::Boolean(false)),
            ("active", rmpv::Value::Boolean(true)),
        ]);
        assert!(evaluate_predicate(
            &node,
            &EvalContext { auth: Some(&auth), data: &data_active_owner }
        ));

        // Non-owner, public, active — true.
        let data_public_active = make_data(&[
            ("ownerId", rmpv::Value::String("u2".into())),
            ("public", rmpv::Value::Boolean(true)),
            ("active", rmpv::Value::Boolean(true)),
        ]);
        let auth2 = make_auth("u1", &[]);
        assert!(evaluate_predicate(
            &node,
            &EvalContext { auth: Some(&auth2), data: &data_public_active }
        ));
    }

    // ---- null literal parser tests (AC8, AC9, AC10, AC15) ----

    /// AC8: `auth == null` produces IsNull node with attribute "auth".
    #[test]
    fn null_rhs_eq_produces_is_null() {
        let node = parsed("auth == null");
        assert_eq!(node.op, PredicateOp::IsNull);
        assert_eq!(node.attribute.as_deref(), Some("auth"));
        assert!(node.value.is_none());
        assert!(node.value_ref.is_none());
    }

    /// AC9: `auth != null` produces IsNotNull node with attribute "auth".
    #[test]
    fn null_rhs_neq_produces_is_not_null() {
        let node = parsed("auth != null");
        assert_eq!(node.op, PredicateOp::IsNotNull);
        assert_eq!(node.attribute.as_deref(), Some("auth"));
    }

    /// AC10: `data.public == true && auth == null` produces And node with correct children.
    #[test]
    fn null_rhs_in_and_expression() {
        let node = parsed("data.public == true && auth == null");
        assert_eq!(node.op, PredicateOp::And);
        let children = node.children.as_ref().expect("And node must have children");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].op, PredicateOp::Eq);
        assert_eq!(children[0].attribute.as_deref(), Some("public"));
        assert_eq!(children[1].op, PredicateOp::IsNull);
        assert_eq!(children[1].attribute.as_deref(), Some("auth"));
    }

    /// `auth.id == null` produces IsNull node with attribute "auth.id".
    #[test]
    fn null_rhs_dotted_auth_attribute() {
        let node = parsed("auth.id == null");
        assert_eq!(node.op, PredicateOp::IsNull);
        assert_eq!(node.attribute.as_deref(), Some("auth.id"));
    }

    /// `data.field == null` produces IsNull node with attribute "field" (data. prefix stripped).
    #[test]
    fn null_rhs_data_attribute_strips_prefix() {
        let node = parsed("data.field == null");
        assert_eq!(node.op, PredicateOp::IsNull);
        assert_eq!(node.attribute.as_deref(), Some("field"));
    }

    /// AC15: `null == auth` returns a parse error (null is not valid as LHS).
    #[test]
    fn null_lhs_returns_parse_error() {
        let err = parse_err("null == auth");
        assert!(
            err.message.contains("null literal is not valid as left-hand side"),
            "unexpected error message: {}",
            err.message
        );
    }

    // ---- containsAll parser tests ----

    #[test]
    fn contains_all_field_ref_rhs() {
        // AC1: data.tags containsAll auth.requiredTags
        let node = parsed("data.tags containsAll auth.requiredTags");
        assert_eq!(node.op, PredicateOp::ContainsAll);
        assert_eq!(node.attribute, Some("tags".to_string()));
        assert_eq!(node.value_ref, Some("auth.requiredTags".to_string()));
        assert!(node.value.is_none());
        assert!(node.children.is_none());
    }

    #[test]
    fn contains_all_string_literal_rhs() {
        // data.tags containsAll 'admin' -> scalar literal RHS
        let node = parsed("data.tags containsAll 'admin'");
        assert_eq!(node.op, PredicateOp::ContainsAll);
        assert_eq!(node.attribute, Some("tags".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::String("admin".into())));
        assert!(node.value_ref.is_none());
    }

    #[test]
    fn contains_all_non_data_lhs_is_error() {
        // AC12: auth.roles containsAll auth.x -> parse error
        let err = parse_err("auth.roles containsAll auth.x");
        assert!(
            err.message.contains("left side of 'containsAll' must be a data.*"),
            "unexpected error: {err}"
        );
    }

    // ---- containsAny parser tests ----

    #[test]
    fn contains_any_field_ref_rhs() {
        // AC2: data.roles containsAny auth.allowedRoles
        let node = parsed("data.roles containsAny auth.allowedRoles");
        assert_eq!(node.op, PredicateOp::ContainsAny);
        assert_eq!(node.attribute, Some("roles".to_string()));
        assert_eq!(node.value_ref, Some("auth.allowedRoles".to_string()));
        assert!(node.value.is_none());
    }

    #[test]
    fn contains_any_non_data_lhs_is_error() {
        let err = parse_err("auth.roles containsAny auth.x");
        assert!(
            err.message.contains("left side of 'containsAny' must be a data.*"),
            "unexpected error: {err}"
        );
    }

    // ---- startsWith parser tests ----

    #[test]
    fn starts_with_string_literal() {
        // AC3: data.path startsWith '/public'
        let node = parsed("data.path startsWith '/public'");
        assert_eq!(node.op, PredicateOp::StartsWith);
        assert_eq!(node.attribute, Some("path".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::String("/public".into())));
        assert!(node.value_ref.is_none());
    }

    #[test]
    fn starts_with_field_ref_rhs() {
        // data.path startsWith auth.pathPrefix -> value_ref
        let node = parsed("data.path startsWith auth.pathPrefix");
        assert_eq!(node.op, PredicateOp::StartsWith);
        assert_eq!(node.attribute, Some("path".to_string()));
        assert_eq!(node.value_ref, Some("auth.pathPrefix".to_string()));
        assert!(node.value.is_none());
    }

    #[test]
    fn starts_with_non_data_lhs_is_error() {
        let err = parse_err("auth.path startsWith '/public'");
        assert!(
            err.message.contains("left side of 'startsWith' must be a data.*"),
            "unexpected error: {err}"
        );
    }

    // ---- endsWith parser tests ----

    #[test]
    fn ends_with_string_literal() {
        // AC4: data.email endsWith '@company.com'
        let node = parsed("data.email endsWith '@company.com'");
        assert_eq!(node.op, PredicateOp::EndsWith);
        assert_eq!(node.attribute, Some("email".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::String("@company.com".into())));
        assert!(node.value_ref.is_none());
    }

    #[test]
    fn ends_with_non_data_lhs_is_error() {
        let err = parse_err("auth.email endsWith '@company.com'");
        assert!(
            err.message.contains("left side of 'endsWith' must be a data.*"),
            "unexpected error: {err}"
        );
    }

    // ---- keyword disambiguation: "contains" vs "containsAll"/"containsAny" ----

    #[test]
    fn contains_keyword_unchanged_after_new_keywords() {
        // Existing "contains" keyword must still work correctly (AC13)
        let node = parsed("data.tags contains 'vip'");
        assert_eq!(node.op, PredicateOp::In);
        assert_eq!(node.attribute, Some("tags".to_string()));
        assert_eq!(node.value, Some(rmpv::Value::String("vip".into())));
    }

    // ---- updated catch-all error message ----

    #[test]
    fn catch_all_error_includes_new_keywords() {
        let err = parse_err("data.x");
        assert!(
            err.message.contains("containsAll"),
            "error message should mention 'containsAll': {err}"
        );
        assert!(
            err.message.contains("startsWith"),
            "error message should mention 'startsWith': {err}"
        );
    }

    // ---- new operators in compound expressions ----

    #[test]
    fn contains_all_and_starts_with_compound() {
        // AC11 parser side: data.tags containsAll auth.required && data.path startsWith '/api'
        let node = parsed("data.tags containsAll auth.required && data.path startsWith '/api'");
        assert_eq!(node.op, PredicateOp::And);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 2);

        let ca_node = &children[0];
        assert_eq!(ca_node.op, PredicateOp::ContainsAll);
        assert_eq!(ca_node.attribute, Some("tags".to_string()));
        assert_eq!(ca_node.value_ref, Some("auth.required".to_string()));

        let sw_node = &children[1];
        assert_eq!(sw_node.op, PredicateOp::StartsWith);
        assert_eq!(sw_node.attribute, Some("path".to_string()));
        assert_eq!(sw_node.value, Some(rmpv::Value::String("/api".into())));
    }

    // ---- Round-trip integration tests (G4): parse + evaluate ----

    fn make_array_data(key: &str, values: &[&str]) -> rmpv::Value {
        make_data(&[(
            key,
            rmpv::Value::Array(
                values
                    .iter()
                    .map(|s| rmpv::Value::String((*s).into()))
                    .collect(),
            ),
        )])
    }

    // ---- containsAll round-trip ----

    #[test]
    fn roundtrip_contains_all_field_ref_true() {
        // AC1 + AC5: data.tags containsAll auth.requiredTags -> true when all present
        let node = parsed("data.tags containsAll auth.requiredTags");
        let data = make_array_data("tags", &["a", "b", "c"]);
        let auth = make_data(&[(
            "requiredTags",
            rmpv::Value::Array(vec![
                rmpv::Value::String("a".into()),
                rmpv::Value::String("c".into()),
            ]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_contains_all_field_ref_false() {
        // AC5: false when expected has an element not in actual
        let node = parsed("data.tags containsAll auth.requiredTags");
        let data = make_array_data("tags", &["a", "b", "c"]);
        let auth = make_data(&[(
            "requiredTags",
            rmpv::Value::Array(vec![
                rmpv::Value::String("a".into()),
                rmpv::Value::String("d".into()),
            ]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(!evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_contains_all_scalar_literal_rhs() {
        // AC14: data.tags containsAll 'admin' — scalar literal treated as single-element array
        let node = parsed("data.tags containsAll 'admin'");
        let data_yes = make_array_data("tags", &["admin", "editor"]);
        let data_no = make_array_data("tags", &["editor", "viewer"]);
        assert!(evaluate_predicate(&node, &EvalContext::data_only(&data_yes)));
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data_no)));
    }

    #[test]
    fn roundtrip_contains_all_non_array_actual_false() {
        // AC9: non-array actual -> false (safe deny)
        let node = parsed("data.tags containsAll 'admin'");
        let data = make_data(&[("tags", rmpv::Value::String("admin".into()))]);
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    // ---- containsAny round-trip ----

    #[test]
    fn roundtrip_contains_any_field_ref_true() {
        // AC2 + AC6: data.roles containsAny auth.allowedRoles -> true when overlap exists
        let node = parsed("data.roles containsAny auth.allowedRoles");
        let data = make_array_data("roles", &["a", "b"]);
        let auth = make_data(&[(
            "allowedRoles",
            rmpv::Value::Array(vec![
                rmpv::Value::String("b".into()),
                rmpv::Value::String("x".into()),
            ]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_contains_any_field_ref_false() {
        // AC6: false when no overlap
        let node = parsed("data.roles containsAny auth.allowedRoles");
        let data = make_array_data("roles", &["a", "b"]);
        let auth = make_data(&[(
            "allowedRoles",
            rmpv::Value::Array(vec![
                rmpv::Value::String("x".into()),
                rmpv::Value::String("y".into()),
            ]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(!evaluate_predicate(&node, &ctx));
    }

    // ---- startsWith round-trip ----

    #[test]
    fn roundtrip_starts_with_true() {
        // AC3 + AC7: data.path startsWith '/public' -> true
        let node = parsed("data.path startsWith '/public'");
        let data = make_data(&[("path", rmpv::Value::String("/public/docs".into()))]);
        assert!(evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    #[test]
    fn roundtrip_starts_with_false() {
        // AC7: false for non-matching prefix
        let node = parsed("data.path startsWith '/public'");
        let data = make_data(&[("path", rmpv::Value::String("/private/docs".into()))]);
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    #[test]
    fn roundtrip_starts_with_field_ref() {
        // data.path startsWith auth.pathPrefix
        let node = parsed("data.path startsWith auth.pathPrefix");
        let data = make_data(&[("path", rmpv::Value::String("/api/v2/users".into()))]);
        let auth = make_data(&[("pathPrefix", rmpv::Value::String("/api/v2".into()))]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(evaluate_predicate(&node, &ctx));
    }

    // ---- endsWith round-trip ----

    #[test]
    fn roundtrip_ends_with_true() {
        // AC4 + AC8: data.email endsWith '@company.com' -> true
        let node = parsed("data.email endsWith '@company.com'");
        let data = make_data(&[("email", rmpv::Value::String("user@company.com".into()))]);
        assert!(evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    #[test]
    fn roundtrip_ends_with_false() {
        // AC8: false for non-matching suffix
        let node = parsed("data.email endsWith '@company.com'");
        let data = make_data(&[("email", rmpv::Value::String("user@other.com".into()))]);
        assert!(!evaluate_predicate(&node, &EvalContext::data_only(&data)));
    }

    // ---- compound expression round-trip (AC11) ----

    #[test]
    fn roundtrip_contains_all_and_starts_with_compound_true() {
        // AC11: data.tags containsAll auth.required && data.path startsWith '/api'
        let node = parsed(
            "data.tags containsAll auth.required && data.path startsWith '/api'",
        );
        let data = make_data(&[
            (
                "tags",
                rmpv::Value::Array(vec![
                    rmpv::Value::String("x".into()),
                    rmpv::Value::String("y".into()),
                ]),
            ),
            ("path", rmpv::Value::String("/api/users".into())),
        ]);
        let auth = make_data(&[(
            "required",
            rmpv::Value::Array(vec![rmpv::Value::String("x".into())]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_contains_all_and_starts_with_compound_false_path() {
        // AC11: false when path condition fails
        let node = parsed(
            "data.tags containsAll auth.required && data.path startsWith '/api'",
        );
        let data = make_data(&[
            (
                "tags",
                rmpv::Value::Array(vec![rmpv::Value::String("x".into())]),
            ),
            ("path", rmpv::Value::String("/public/docs".into())),
        ]);
        let auth = make_data(&[(
            "required",
            rmpv::Value::Array(vec![rmpv::Value::String("x".into())]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(!evaluate_predicate(&node, &ctx));
    }

    #[test]
    fn roundtrip_three_operator_compound() {
        // Validation Checklist item 4: 3-child And node with containsAll, startsWith, ==
        let node = parsed(
            "data.tags containsAll auth.required && data.path startsWith '/api' && data.status == 'active'",
        );
        assert_eq!(node.op, PredicateOp::And);
        let children = node.children.as_ref().unwrap();
        assert_eq!(children.len(), 3, "expected 3 children in flattened And node");

        // Evaluate: all conditions true
        let data = make_data(&[
            (
                "tags",
                rmpv::Value::Array(vec![rmpv::Value::String("req".into())]),
            ),
            ("path", rmpv::Value::String("/api/v1".into())),
            ("status", rmpv::Value::String("active".into())),
        ]);
        let auth = make_data(&[(
            "required",
            rmpv::Value::Array(vec![rmpv::Value::String("req".into())]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data };
        assert!(evaluate_predicate(&node, &ctx));

        // Evaluate: status wrong -> false
        let data_bad = make_data(&[
            (
                "tags",
                rmpv::Value::Array(vec![rmpv::Value::String("req".into())]),
            ),
            ("path", rmpv::Value::String("/api/v1".into())),
            ("status", rmpv::Value::String("inactive".into())),
        ]);
        assert!(!evaluate_predicate(&node, &EvalContext { auth: Some(&auth), data: &data_bad }));
    }

    #[test]
    fn roundtrip_contains_any_or_ends_with() {
        // data.roles containsAny auth.allowed || data.email endsWith '@admin.com'
        let node = parsed(
            "data.roles containsAny auth.allowed || data.email endsWith '@admin.com'",
        );
        // True via containsAny
        let data_via_roles = make_data(&[
            (
                "roles",
                rmpv::Value::Array(vec![rmpv::Value::String("mod".into())]),
            ),
            ("email", rmpv::Value::String("user@company.com".into())),
        ]);
        let auth = make_data(&[(
            "allowed",
            rmpv::Value::Array(vec![rmpv::Value::String("mod".into())]),
        )]);
        let ctx = EvalContext { auth: Some(&auth), data: &data_via_roles };
        assert!(evaluate_predicate(&node, &ctx));

        // True via endsWith
        let data_via_email = make_data(&[
            (
                "roles",
                rmpv::Value::Array(vec![rmpv::Value::String("guest".into())]),
            ),
            ("email", rmpv::Value::String("super@admin.com".into())),
        ]);
        let ctx2 = EvalContext { auth: Some(&auth), data: &data_via_email };
        assert!(evaluate_predicate(&node, &ctx2));

        // False on both
        let data_neither = make_data(&[
            (
                "roles",
                rmpv::Value::Array(vec![rmpv::Value::String("guest".into())]),
            ),
            ("email", rmpv::Value::String("user@company.com".into())),
        ]);
        let ctx3 = EvalContext { auth: Some(&auth), data: &data_neither };
        assert!(!evaluate_predicate(&node, &ctx3));
    }
}
