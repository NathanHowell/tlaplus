//! TLA+ module parser built on top of `winnow`.
//!
//! The parser is intentionally split into a lexing stage that tokenises source
//! text and a syntax stage that consumes those tokens to build a lightweight AST.
//! This approach mirrors the [`winnow` lexing guide][winnow-lex] and gives later
//! phases (semantic evaluation, exploration) a stable representation to extend.
//!
//! [winnow-lex]: https://docs.rs/winnow/latest/winnow/_topic/lexing/index.html

pub use ast::{
    AstNode, Declaration, Expression, Identifier, Module, OperatorDefinition, OperatorParameter,
    Span, TokenTree,
};
pub use error::{ParseError, ParseErrorKind};
pub use syntax::{parse_module, ParseResult, ParserOptions};

mod ast;
mod error;
mod lexer;
mod syntax;

/// Convenient parser result alias.
pub type Result<T> = std::result::Result<T, ParseError>;
