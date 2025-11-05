use std::fmt;

use super::lexer::Token;

/// Half-open byte range describing an AST node's position in the source module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn union(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Shared trait for AST nodes that occupy a source span.
pub trait AstNode {
    fn span(&self) -> Span;
}

/// Identifier with its source span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub name: String,
    pub span: Span,
}

impl Identifier {
    pub fn new<S: Into<String>>(name: S, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

impl AstNode for Identifier {
    fn span(&self) -> Span {
        self.span
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name.fmt(f)
    }
}

/// Container for a parsed module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: Identifier,
    pub extends: Vec<Identifier>,
    pub declarations: Vec<Declaration>,
    pub span: Span,
}

impl AstNode for Module {
    fn span(&self) -> Span {
        self.span
    }
}

/// High-level declarations supported by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    Constants {
        names: Vec<Identifier>,
        span: Span,
    },
    Variables {
        names: Vec<Identifier>,
        span: Span,
    },
    Operator(OperatorDefinition),
    Theorem {
        name: Option<Identifier>,
        body: Expression,
        span: Span,
    },
    Assume {
        name: Option<Identifier>,
        body: Expression,
        span: Span,
    },
    Other {
        keyword: String,
        body: Expression,
        span: Span,
    },
}

impl AstNode for Declaration {
    fn span(&self) -> Span {
        match self {
            Declaration::Constants { span, .. }
            | Declaration::Variables { span, .. }
            | Declaration::Theorem { span, .. }
            | Declaration::Assume { span, .. }
            | Declaration::Other { span, .. } => *span,
            Declaration::Operator(def) => def.span,
        }
    }
}

/// Operator parameter representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorParameter {
    pub name: Identifier,
    pub tuple: bool,
}

impl AstNode for OperatorParameter {
    fn span(&self) -> Span {
        self.name.span
    }
}

/// Operator definition AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorDefinition {
    pub name: Identifier,
    pub params: Vec<OperatorParameter>,
    pub body: Expression,
    pub local: bool,
    pub span: Span,
}

impl AstNode for OperatorDefinition {
    fn span(&self) -> Span {
        self.span
    }
}

/// Expression captured from the source, represented as a token tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression {
    pub span: Span,
    pub source: String,
    pub tokens: TokenTree,
}

impl AstNode for Expression {
    fn span(&self) -> Span {
        self.span
    }
}

/// Sequence of tokens that form an expression.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenTree {
    pub tokens: Vec<Token>,
}

impl TokenTree {
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}
