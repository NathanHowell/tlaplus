use std::fmt;

use super::ast::Span;

/// Specific failure category surfaced by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    Lexical {
        message: String,
    },
    UnexpectedToken {
        expected: &'static str,
        found: Option<String>,
    },
    UnexpectedEof {
        expected: &'static str,
    },
    UnterminatedModule,
    Custom(String),
}

/// Parser error with span information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

impl ParseError {
    pub fn lexical<S: Into<String>>(span: Span, message: S) -> Self {
        Self {
            kind: ParseErrorKind::Lexical {
                message: message.into(),
            },
            span,
        }
    }

    pub fn unexpected_token<S: Into<String>>(
        span: Span,
        expected: &'static str,
        found: Option<S>,
    ) -> Self {
        Self {
            kind: ParseErrorKind::UnexpectedToken {
                expected,
                found: found.map(Into::into),
            },
            span,
        }
    }

    pub fn unexpected_eof(span: Span, expected: &'static str) -> Self {
        Self {
            kind: ParseErrorKind::UnexpectedEof { expected },
            span,
        }
    }

    pub fn unterminated_module(span: Span) -> Self {
        Self {
            kind: ParseErrorKind::UnterminatedModule,
            span,
        }
    }

    pub fn custom<S: Into<String>>(span: Span, message: S) -> Self {
        Self {
            kind: ParseErrorKind::Custom(message.into()),
            span,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ParseErrorKind::Lexical { message } => write!(f, "lexing failed: {}", message),
            ParseErrorKind::UnexpectedToken { expected, found } => match found {
                Some(found) => write!(f, "expected {}, found {}", expected, found),
                None => write!(f, "expected {}, found end of input", expected),
            },
            ParseErrorKind::UnexpectedEof { expected } => {
                write!(f, "expected {}, found end of input", expected)
            }
            ParseErrorKind::UnterminatedModule => write!(f, "unterminated module"),
            ParseErrorKind::Custom(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ParseError {}
