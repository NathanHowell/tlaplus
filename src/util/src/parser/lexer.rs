use std::iter::Peekable;
use std::str::CharIndices;

use super::ast::Span;
use super::error::ParseError;
use super::error::ParseErrorKind;
use winnow::stream::Location;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keyword {
    Module,
    Extends,
    Constant,
    Constants,
    Variable,
    Variables,
    Theorem,
    Assume,
    Lemma,
    Axiom,
    Local,
    Define,
    Fairness,
}

impl Keyword {
    fn from_ident(ident: &str) -> Option<Self> {
        match ident {
            "MODULE" => Some(Self::Module),
            "EXTENDS" => Some(Self::Extends),
            "CONSTANT" => Some(Self::Constant),
            "CONSTANTS" => Some(Self::Constants),
            "VARIABLE" => Some(Self::Variable),
            "VARIABLES" => Some(Self::Variables),
            "THEOREM" => Some(Self::Theorem),
            "ASSUME" => Some(Self::Assume),
            "LEMMA" => Some(Self::Lemma),
            "AXIOM" => Some(Self::Axiom),
            "LOCAL" => Some(Self::Local),
            "DEFINE" => Some(Self::Define),
            "FAIRNESS" => Some(Self::Fairness),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Dot,
    Colon,
    Semicolon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Keyword(Keyword),
    Identifier,
    Number,
    String,
    Operator(String),
    Symbol(SymbolKind),
    Newline,
    ModuleDelimiter,
    ModuleEnd,
    Unknown,
    Eof,
}

/// Lexed token with textual representation and span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: Span,
    pub line: usize,
    pub column: usize,
}

impl Token {
    fn new(kind: TokenKind, lexeme: String, span: Span) -> Self {
        Self {
            kind,
            lexeme,
            span,
            line: 0,
            column: 0,
        }
    }

    pub fn describe(&self) -> String {
        match &self.kind {
            TokenKind::Keyword(_) => format!("keyword '{}'", self.lexeme),
            TokenKind::Identifier => format!("identifier '{}'", self.lexeme),
            TokenKind::Number => format!("number '{}'", self.lexeme),
            TokenKind::String => "string literal".to_string(),
            TokenKind::Operator(op) => format!("operator '{}'", op),
            TokenKind::Symbol(_) => format!("symbol '{}'", self.lexeme),
            TokenKind::ModuleDelimiter => "module delimiter".to_string(),
            TokenKind::ModuleEnd => "module end marker".to_string(),
            TokenKind::Newline => "newline".to_string(),
            TokenKind::Unknown => format!("token '{}'", self.lexeme),
            TokenKind::Eof => "end of file".to_string(),
        }
    }
}

impl Location for Token {
    fn previous_token_end(&self) -> usize {
        self.span.end
    }

    fn current_token_start(&self) -> usize {
        self.span.start
    }
}

/// Tokenise the provided module source.
pub fn lex(source: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut chars = source.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        match ch {
            ' ' | '\t' | '\u{000B}' | '\u{000C}' | '\r' => continue,
            '\n' => {
                tokens.push(Token::new(
                    TokenKind::Newline,
                    "\n".to_string(),
                    Span::new(idx, idx + 1),
                ));
            }
            '-' if source[idx..].starts_with("----") => {
                consume_span(&mut chars, 3);
                tokens.push(Token::new(
                    TokenKind::ModuleDelimiter,
                    "----".to_string(),
                    Span::new(idx, idx + 4),
                ));
            }
            '=' if source[idx..].starts_with("====") => {
                consume_span(&mut chars, 3);
                tokens.push(Token::new(
                    TokenKind::ModuleEnd,
                    "====".to_string(),
                    Span::new(idx, idx + 4),
                ));
            }
            '\\' if source[idx..].starts_with("\\*") => {
                consume_comment_line(idx, &mut chars, source)?;
            }
            '(' if source[idx..].starts_with("(*") => {
                consume_block_comment(idx, &mut chars, source)?;
            }
            '"' => {
                let (lexeme, end) = consume_string(idx, &mut chars, source)?;
                tokens.push(Token::new(TokenKind::String, lexeme, Span::new(idx, end)));
            }
            ch if is_identifier_start(ch) => {
                let (ident, end) = consume_identifier(idx, ch, &mut chars, source);
                if let Some(keyword) = Keyword::from_ident(&ident) {
                    tokens.push(Token::new(
                        TokenKind::Keyword(keyword),
                        ident,
                        Span::new(idx, end),
                    ));
                } else {
                    tokens.push(Token::new(
                        TokenKind::Identifier,
                        ident,
                        Span::new(idx, end),
                    ));
                }
            }
            ch if ch.is_ascii_digit() => {
                let (number, end) = consume_number(idx, ch, &mut chars, source);
                tokens.push(Token::new(TokenKind::Number, number, Span::new(idx, end)));
            }
            ch if is_operator_char(ch) => {
                let (operator, end) = consume_operator(idx, ch, &mut chars, source);
                tokens.push(Token::new(
                    TokenKind::Operator(operator.clone()),
                    operator,
                    Span::new(idx, end),
                ));
            }
            '(' => tokens.push(symbol_token(idx, SymbolKind::LParen)),
            ')' => tokens.push(symbol_token(idx, SymbolKind::RParen)),
            '[' => tokens.push(symbol_token(idx, SymbolKind::LBracket)),
            ']' => tokens.push(symbol_token(idx, SymbolKind::RBracket)),
            '{' => tokens.push(symbol_token(idx, SymbolKind::LBrace)),
            '}' => tokens.push(symbol_token(idx, SymbolKind::RBrace)),
            ',' => tokens.push(symbol_token(idx, SymbolKind::Comma)),
            '.' => tokens.push(symbol_token(idx, SymbolKind::Dot)),
            ':' => tokens.push(symbol_token(idx, SymbolKind::Colon)),
            ';' => tokens.push(symbol_token(idx, SymbolKind::Semicolon)),
            _ => tokens.push(Token::new(
                TokenKind::Unknown,
                ch.to_string(),
                Span::new(idx, idx + ch.len_utf8()),
            )),
        }
    }

    assign_positions(source, &mut tokens);

    let end_index = source.len();
    tokens.push(Token::new(
        TokenKind::Eof,
        String::new(),
        Span::new(end_index, end_index),
    ));

    Ok(tokens)
}

fn symbol_token(idx: usize, kind: SymbolKind) -> Token {
    Token::new(
        TokenKind::Symbol(kind),
        match kind {
            SymbolKind::LParen => "(",
            SymbolKind::RParen => ")",
            SymbolKind::LBracket => "[",
            SymbolKind::RBracket => "]",
            SymbolKind::LBrace => "{",
            SymbolKind::RBrace => "}",
            SymbolKind::Comma => ",",
            SymbolKind::Dot => ".",
            SymbolKind::Colon => ":",
            SymbolKind::Semicolon => ";",
        }
        .to_string(),
        Span::new(idx, idx + 1),
    )
}

fn consume_span(chars: &mut Peekable<CharIndices<'_>>, count: usize) {
    for _ in 0..count {
        let _ = chars.next();
    }
}

fn consume_comment_line(
    start: usize,
    chars: &mut Peekable<CharIndices<'_>>,
    source: &str,
) -> Result<(), ParseError> {
    let mut end = start + 2;
    while let Some(&(idx, ch)) = chars.peek() {
        if ch == '\n' {
            break;
        }
        end = idx + ch.len_utf8();
        chars.next();
    }
    if end > source.len() {
        return Err(ParseError::lexical(
            Span::new(start, source.len()),
            "unterminated comment",
        ));
    }
    Ok(())
}

fn consume_block_comment(
    start: usize,
    chars: &mut Peekable<CharIndices<'_>>,
    source: &str,
) -> Result<(), ParseError> {
    let mut depth = 1usize;
    while let Some((idx, ch)) = chars.next() {
        let slice = &source[idx..];
        if ch == '(' && slice.starts_with("(*") {
            depth += 1;
            consume_span(chars, 1);
            continue;
        }
        if ch == '*' && slice.starts_with("*)") {
            depth -= 1;
            consume_span(chars, 1);
            if depth == 0 {
                return Ok(());
            }
            continue;
        }
    }
    Err(ParseError {
        kind: ParseErrorKind::Lexical {
            message: "unterminated block comment".to_string(),
        },
        span: Span::new(start, source.len()),
    })
}

fn consume_string(
    start: usize,
    chars: &mut Peekable<CharIndices<'_>>,
    source: &str,
) -> Result<(String, usize), ParseError> {
    let mut end = start + 1;
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '"' => {
                end = idx + 1;
                return Ok((source[start..end].to_string(), end));
            }
            '\\' => {
                if let Some((next_idx, next_ch)) = chars.next() {
                    end = next_idx + next_ch.len_utf8();
                } else {
                    break;
                }
            }
            _ => {
                end = idx + ch.len_utf8();
            }
        }
    }
    Err(ParseError::lexical(
        Span::new(start, end.max(start)),
        "unterminated string literal",
    ))
}

fn consume_identifier(
    start: usize,
    first: char,
    chars: &mut Peekable<CharIndices<'_>>,
    source: &str,
) -> (String, usize) {
    let mut end = start + first.len_utf8();
    while let Some(&(idx, ch)) = chars.peek() {
        if is_identifier_continue(ch) {
            chars.next();
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    (source[start..end].to_string(), end)
}

fn consume_number(
    start: usize,
    first: char,
    chars: &mut Peekable<CharIndices<'_>>,
    source: &str,
) -> (String, usize) {
    let mut end = start + first.len_utf8();
    while let Some(&(idx, ch)) = chars.peek() {
        if ch.is_ascii_digit() {
            chars.next();
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    (source[start..end].to_string(), end)
}

fn consume_operator(
    start: usize,
    first: char,
    chars: &mut Peekable<CharIndices<'_>>,
    source: &str,
) -> (String, usize) {
    let mut end = start + first.len_utf8();
    while let Some(&(idx, ch)) = chars.peek() {
        if is_operator_char(ch) {
            chars.next();
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    (source[start..end].to_string(), end)
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit() || ch == '\''
}

fn is_operator_char(ch: char) -> bool {
    matches!(
        ch,
        '+' | '-'
            | '*'
            | '/'
            | '\\'
            | '='
            | '>'
            | '<'
            | '~'
            | '!'
            | '@'
            | '#'
            | '$'
            | '%'
            | '^'
            | '&'
            | '|'
            | '`'
    )
}

fn assign_positions(source: &str, tokens: &mut [Token]) {
    let line_offsets = compute_line_offsets(source);
    for token in tokens {
        let (line, column) = offset_to_line_column(&line_offsets, token.span.start);
        token.line = line;
        token.column = column;
    }
}

fn compute_line_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in source.char_indices() {
        if ch == '\n' {
            offsets.push(idx + ch.len_utf8());
        }
    }
    offsets
}

fn offset_to_line_column(offsets: &[usize], index: usize) -> (usize, usize) {
    if offsets.is_empty() {
        return (1, index);
    }
    let mut line = offsets.partition_point(|&offset| offset <= index);
    if line == 0 {
        return (1, index);
    }
    let line_start = offsets[line - 1];
    if line > offsets.len() {
        line = offsets.len();
    }
    (line, index - line_start)
}
