use super::ast::{
    AstNode, Declaration, Expression, Identifier, Module, OperatorDefinition, OperatorParameter,
    Span, TokenTree,
};
use super::error::ParseError;
use super::lexer::{self, Keyword, SymbolKind, Token, TokenKind};
use winnow::stream::{Stream, TokenSlice};

pub type ParseResult<T> = Result<T, ParseError>;

#[derive(Debug, Clone, Copy, Default)]
pub struct ParserOptions {
    /// Reserved for future configuration (e.g., comment preservation).
    pub preserve_comments: bool,
}

pub fn parse_module(source: &str, options: ParserOptions) -> ParseResult<Module> {
    let tokens = lexer::lex(source)?;
    Parser::new(source, tokens, options).parse()
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    index: usize,
    _options: ParserOptions,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, tokens: Vec<Token>, options: ParserOptions) -> Self {
        Self {
            source,
            tokens,
            index: 0,
            _options: options,
        }
    }

    fn parse(mut self) -> ParseResult<Module> {
        self.consume_newlines();

        let header_start = self
            .expect_token(TokenKind::ModuleDelimiter, "module delimiter")?
            .span;
        self.consume_newlines();
        self.expect_keyword(Keyword::Module)?;
        let name = self.expect_identifier()?;
        self.consume_newlines();
        let header_end = self
            .expect_token(TokenKind::ModuleDelimiter, "module delimiter")?
            .span;
        self.consume_newlines();

        let extends = if self.peek_keyword(Keyword::Extends) {
            self.advance(); // consume EXTENDS
            self.parse_identifier_list()?
        } else {
            Vec::new()
        };

        self.consume_blank_lines();

        let mut declarations = Vec::new();
        loop {
            if self.peek_kind(TokenKind::ModuleEnd) {
                let _ = self.advance();
                break;
            }
            if self.peek_kind(TokenKind::Eof) {
                return Err(ParseError::unterminated_module(
                    header_start.union(header_end),
                ));
            }
            let decl = self.parse_declaration()?;
            declarations.push(decl);
            self.consume_blank_lines();
        }

        let module_span = if let Some(last_decl) = declarations.last() {
            header_start.union(last_decl.span())
        } else {
            header_start.union(header_end)
        };

        Ok(Module {
            name,
            extends,
            declarations,
            span: module_span,
        })
    }

    fn parse_declaration(&mut self) -> ParseResult<Declaration> {
        if self.peek_keyword(Keyword::Constant) || self.peek_keyword(Keyword::Constants) {
            let keyword_span = self.advance().span;
            let names = self.parse_identifier_list()?;
            let span = names
                .last()
                .map(|id| keyword_span.union(id.span))
                .unwrap_or(keyword_span);
            return Ok(Declaration::Constants { names, span });
        }

        if self.peek_keyword(Keyword::Variable) || self.peek_keyword(Keyword::Variables) {
            let keyword_span = self.advance().span;
            let names = self.parse_identifier_list()?;
            let span = names
                .last()
                .map(|id| keyword_span.union(id.span))
                .unwrap_or(keyword_span);
            return Ok(Declaration::Variables { names, span });
        }

        if self.peek_keyword(Keyword::Assume) {
            let keyword = self.advance();
            let (name, body) = self.parse_named_expression(true)?;
            let span = keyword.span.union(body.span);
            return Ok(Declaration::Assume { name, body, span });
        }

        if self.peek_keyword(Keyword::Theorem)
            || self.peek_keyword(Keyword::Lemma)
            || self.peek_keyword(Keyword::Axiom)
        {
            let keyword = self.advance();
            let (name, body) = self.parse_named_expression(false)?;
            let span = keyword.span.union(body.span);
            return Ok(Declaration::Theorem { name, body, span });
        }

        if self.peek_keyword(Keyword::Define) || self.peek_keyword(Keyword::Fairness) {
            let keyword_token = self.advance();
            let body = self.parse_expression()?;
            let span = keyword_token.span.union(body.span);
            return Ok(Declaration::Other {
                keyword: keyword_token.lexeme.clone(),
                body,
                span,
            });
        }

        let local = if self.peek_keyword(Keyword::Local) {
            self.advance();
            true
        } else {
            false
        };

        if self.peek_kind(TokenKind::Identifier) {
            return self.parse_operator_definition(local);
        }

        let token = self
            .peek()
            .cloned()
            .or_else(|| self.tokens.last().cloned())
            .unwrap_or_else(|| Token {
                kind: TokenKind::Eof,
                lexeme: String::new(),
                span: Span::new(self.source.len(), self.source.len()),
                line: 0,
                column: 0,
            });
        Err(ParseError::unexpected_token(
            token.span,
            "declaration",
            Some(token.describe()),
        ))
    }

    fn parse_operator_definition(&mut self, local: bool) -> ParseResult<Declaration> {
        let name = self.expect_identifier()?;
        let params = if self.consume_symbol(SymbolKind::LParen) {
            let mut params = Vec::new();
            if !self.peek_symbol(SymbolKind::RParen) {
                loop {
                    let ident = self.expect_identifier()?;
                    params.push(OperatorParameter {
                        name: ident.clone(),
                        tuple: false,
                    });
                    if self.consume_symbol(SymbolKind::Comma) {
                        continue;
                    }
                    break;
                }
            }
            self.expect_symbol(SymbolKind::RParen)?;
            params
        } else {
            Vec::new()
        };
        self.consume_newlines();
        self.expect_definition_operator()?;
        let body = self.parse_expression()?;
        let span = name.span.union(body.span);
        Ok(Declaration::Operator(OperatorDefinition {
            name,
            params,
            body,
            local,
            span,
        }))
    }

    fn parse_named_expression(
        &mut self,
        _name_optional: bool,
    ) -> ParseResult<(Option<Identifier>, Expression)> {
        self.consume_newlines();

        let name = if self.identifier_followed_by_definition() {
            let ident = self.expect_identifier()?;
            self.expect_definition_operator()?;
            Some(ident)
        } else {
            None
        };

        let body = if self.peek_definition_operator() {
            self.expect_definition_operator()?;
            self.parse_expression()?
        } else {
            self.parse_expression()?
        };
        Ok((name, body))
    }

    fn parse_identifier_list(&mut self) -> ParseResult<Vec<Identifier>> {
        self.consume_newlines();
        let mut identifiers = Vec::new();
        loop {
            let ident = self.expect_identifier()?;
            identifiers.push(ident.clone());
            if self.consume_symbol(SymbolKind::Comma) {
                self.consume_newlines();
                continue;
            }
            break;
        }
        Ok(identifiers)
    }

    fn parse_expression(&mut self) -> ParseResult<Expression> {
        let start_index = self.index;
        let mut slice = TokenSlice::new(&self.tokens[self.index..]);
        let mut consumed = 0usize;
        let mut tokens = Vec::new();
        let mut paren = 0isize;
        let mut bracket = 0isize;
        let mut brace = 0isize;

        let mut first_span = None;
        let mut last_span = None;

        while let Some(token_ref) = slice.peek_token() {
            let token = token_ref.clone();

            if matches!(token.kind, TokenKind::Eof) {
                return Err(ParseError::unexpected_eof(token.span, "expression"));
            }

            if matches!(token.kind, TokenKind::ModuleEnd)
                && paren == 0
                && bracket == 0
                && brace == 0
            {
                break;
            }

            let is_newline = matches!(token.kind, TokenKind::Newline);
            let _ = slice.next_token();
            consumed += 1;
            tokens.push(token.clone());
            first_span = first_span.or(Some(token.span));
            last_span = Some(token.span);

            match token.kind {
                TokenKind::Symbol(SymbolKind::LParen) => paren += 1,
                TokenKind::Symbol(SymbolKind::RParen) => paren = (paren - 1).max(0),
                TokenKind::Symbol(SymbolKind::LBracket) => bracket += 1,
                TokenKind::Symbol(SymbolKind::RBracket) => bracket = (bracket - 1).max(0),
                TokenKind::Symbol(SymbolKind::LBrace) => brace += 1,
                TokenKind::Symbol(SymbolKind::RBrace) => brace = (brace - 1).max(0),
                _ => {}
            }

            if is_newline && paren == 0 && bracket == 0 && brace == 0 {
                let rest = &self.tokens[(start_index + consumed)..];
                if let Some(next) = rest.first() {
                    if is_declaration_start(next, rest) {
                        break;
                    }
                }
            }
        }

        if tokens.is_empty() {
            let token = self
                .peek()
                .cloned()
                .or_else(|| self.tokens.last().cloned())
                .unwrap_or_else(|| Token {
                    kind: TokenKind::Eof,
                    lexeme: String::new(),
                    span: Span::new(self.source.len(), self.source.len()),
                    line: 0,
                    column: 0,
                });
            return Err(ParseError::unexpected_token(
                token.span,
                "expression",
                Some(token.describe()),
            ));
        }

        self.index += consumed;

        let span = first_span
            .and_then(|start| last_span.map(|end| start.union(end)))
            .unwrap_or_else(|| Span::new(0, 0));

        let source = if span.start < span.end && span.end <= self.source.len() {
            self.source[span.start..span.end].to_string()
        } else {
            String::new()
        };

        Ok(Expression {
            span,
            source,
            tokens: TokenTree { tokens },
        })
    }

    fn expect_token(
        &mut self,
        expected: TokenKind,
        description: &'static str,
    ) -> ParseResult<Token> {
        let token = self.advance();
        if self.same_kind(&token.kind, &expected) {
            Ok(token)
        } else {
            Err(ParseError::unexpected_token(
                token.span,
                description,
                Some(token.describe()),
            ))
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword) -> ParseResult<Token> {
        let token = self.advance();
        if matches!(token.kind, TokenKind::Keyword(ref k) if *k == keyword) {
            Ok(token)
        } else {
            Err(ParseError::unexpected_token(
                token.span,
                "keyword",
                Some(token.describe()),
            ))
        }
    }

    fn expect_symbol(&mut self, symbol: SymbolKind) -> ParseResult<Token> {
        let token = self.advance();
        if matches!(token.kind, TokenKind::Symbol(s) if s == symbol) {
            Ok(token)
        } else {
            Err(ParseError::unexpected_token(
                token.span,
                "symbol",
                Some(token.describe()),
            ))
        }
    }

    fn expect_identifier(&mut self) -> ParseResult<Identifier> {
        let token = self.advance();
        if matches!(token.kind, TokenKind::Identifier) {
            Ok(Identifier::new(token.lexeme.clone(), token.span))
        } else {
            Err(ParseError::unexpected_token(
                token.span,
                "identifier",
                Some(token.describe()),
            ))
        }
    }

    fn expect_definition_operator(&mut self) -> ParseResult<()> {
        let token = self.peek().cloned();
        if self.peek_definition_operator() {
            let _ = self.advance();
            Ok(())
        } else {
            let token = token
                .or_else(|| self.tokens.last().cloned())
                .unwrap_or_else(|| Token {
                    kind: TokenKind::Eof,
                    lexeme: String::new(),
                    span: Span::new(self.source.len(), self.source.len()),
                    line: 0,
                    column: 0,
                });
            Err(ParseError::unexpected_token(
                token.span,
                "`==`",
                Some(token.describe()),
            ))
        }
    }

    fn peek_definition_operator(&self) -> bool {
        if let Some(token) = self.tokens.get(self.index) {
            matches!(
                &token.kind,
                TokenKind::Operator(op) if op == "==" || op == "="
            )
        } else {
            false
        }
    }

    fn identifier_followed_by_definition(&self) -> bool {
        let Some(token) = self.tokens.get(self.index) else {
            return false;
        };
        if !matches!(token.kind, TokenKind::Identifier) {
            return false;
        }

        let mut idx = self.index + 1;
        let mut depth = 0i32;
        while let Some(next) = self.tokens.get(idx) {
            match &next.kind {
                TokenKind::Operator(op) if (op == "==" || op == "=") && depth == 0 => {
                    return true;
                }
                TokenKind::Symbol(SymbolKind::LParen) => depth += 1,
                TokenKind::Symbol(SymbolKind::RParen) => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::Newline => {
                    if depth == 0 {
                        return false;
                    }
                }
                TokenKind::Keyword(_) | TokenKind::ModuleEnd if depth == 0 => return false,
                TokenKind::Eof => return false,
                _ => {}
            }
            idx += 1;
        }
        false
    }

    fn consume_symbol(&mut self, symbol: SymbolKind) -> bool {
        if self.peek_symbol(symbol) {
            let _ = self.advance();
            true
        } else {
            false
        }
    }

    fn peek_symbol(&self, symbol: SymbolKind) -> bool {
        matches!(
            self.tokens.get(self.index),
            Some(Token {
                kind: TokenKind::Symbol(s),
                ..
            }) if *s == symbol
        )
    }

    fn consume_newlines(&mut self) {
        while self.peek_kind(TokenKind::Newline) {
            let _ = self.advance();
        }
    }

    fn consume_blank_lines(&mut self) {
        loop {
            let initial = self.index;
            self.consume_newlines();
            if self.index == initial {
                break;
            }
        }
    }

    fn peek_keyword(&self, keyword: Keyword) -> bool {
        matches!(
            self.tokens.get(self.index),
            Some(Token {
                kind: TokenKind::Keyword(k),
                ..
            }) if *k == keyword
        )
    }

    fn peek_kind(&self, expected: TokenKind) -> bool {
        matches!(
            self.tokens.get(self.index),
            Some(token) if self.same_kind(&token.kind, &expected)
        )
    }

    fn same_kind(&self, actual: &TokenKind, expected: &TokenKind) -> bool {
        match (actual, expected) {
            (TokenKind::Keyword(a), TokenKind::Keyword(b)) => a == b,
            (TokenKind::Symbol(a), TokenKind::Symbol(b)) => a == b,
            (TokenKind::Operator(a), TokenKind::Operator(b)) => a == b,
            (TokenKind::Identifier, TokenKind::Identifier)
            | (TokenKind::Number, TokenKind::Number)
            | (TokenKind::String, TokenKind::String)
            | (TokenKind::ModuleDelimiter, TokenKind::ModuleDelimiter)
            | (TokenKind::ModuleEnd, TokenKind::ModuleEnd)
            | (TokenKind::Newline, TokenKind::Newline)
            | (TokenKind::Eof, TokenKind::Eof) => true,
            _ => false,
        }
    }

    fn advance(&mut self) -> Token {
        let token = self
            .tokens
            .get(self.index)
            .cloned()
            .unwrap_or_else(|| self.tokens.last().cloned().unwrap());
        self.index = (self.index + 1).min(self.tokens.len());
        token
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }
}

fn is_declaration_start(first: &Token, rest: &[Token]) -> bool {
    if first.column > 0 {
        return false;
    }
    match &first.kind {
        TokenKind::Keyword(
            Keyword::Constant
            | Keyword::Constants
            | Keyword::Variable
            | Keyword::Variables
            | Keyword::Assume
            | Keyword::Theorem
            | Keyword::Lemma
            | Keyword::Axiom
            | Keyword::Define
            | Keyword::Fairness,
        ) => true,
        TokenKind::Keyword(Keyword::Local) => rest
            .get(1)
            .map(|t| matches!(t.kind, TokenKind::Identifier) && t.column == 0)
            .unwrap_or(false),
        TokenKind::Identifier => is_operator_head(rest),
        _ => false,
    }
}

fn is_operator_head(tokens: &[Token]) -> bool {
    let mut index = 1;
    let mut paren_depth = 0isize;
    while let Some(token) = tokens.get(index) {
        match &token.kind {
            TokenKind::Operator(op) if (op == "==" || op == "=") && paren_depth == 0 => {
                return true;
            }
            TokenKind::Symbol(SymbolKind::LParen) => paren_depth += 1,
            TokenKind::Symbol(SymbolKind::RParen) => paren_depth = (paren_depth - 1).max(0),
            TokenKind::Newline => return false,
            TokenKind::ModuleEnd => return true,
            TokenKind::Keyword(_) if paren_depth == 0 => return false,
            _ => {}
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Module {
        parse_module(text, ParserOptions::default()).expect("parse succeeds")
    }

    #[test]
    fn parses_unicode_module() {
        let text = r#"---- MODULE Unicode ----
EXTENDS Naturals, TLC

CONSTANT ΩFlag

VARIABLES 状態, Δcount

Init ==
    /\ 状態 = "開始"

Next ==
    状態' = 状態

THEOREM Spec => []True

===="#;

        let module = parse(text);
        assert_eq!(module.name.name, "Unicode");
        assert_eq!(module.extends.len(), 2);
        assert!(module
            .declarations
            .iter()
            .any(|decl| matches!(decl, Declaration::Constants { .. })));
        assert!(module
            .declarations
            .iter()
            .any(|decl| matches!(decl, Declaration::Variables { .. })));
        let operator_names: Vec<_> = module
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                Declaration::Operator(op) => Some(op.name.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(operator_names.contains(&"Init"));
        assert!(operator_names.contains(&"Next"));
        assert!(module
            .declarations
            .iter()
            .any(|decl| matches!(decl, Declaration::Theorem { .. })));
    }

    #[test]
    fn parses_operator_parameters() {
        let text = r#"---- MODULE Foo ----

Foo(a, b) == a + b

====
"#;

        let module = parse(text);
        match &module.declarations[0] {
            Declaration::Operator(op) => {
                assert_eq!(op.name.name, "Foo");
                assert_eq!(op.params.len(), 2);
                assert!(!op.body.tokens.is_empty());
            }
            _ => panic!("expected operator"),
        }
    }

    #[test]
    fn parses_constants_and_variables() {
        let text = r#"---- MODULE Bar ----
CONSTANTS A, B
VARIABLE X

====
"#;
        let module = parse(text);
        assert!(matches!(
            module.declarations[0],
            Declaration::Constants { .. }
        ));
        assert!(matches!(
            module.declarations[1],
            Declaration::Variables { .. }
        ));
    }
}
