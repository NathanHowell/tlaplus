//! TLC configuration (`MC.cfg`) parsing utilities.
//!
//! Parsing is split into two distinct phases:
//! 1. **Lexing**: Convert the raw file into a stream of line tokens with directive
//!    metadata (keyword, indentation, body) while applying comment stripping.
//! 2. **Parsing**: Interpret the token stream according to TLC configuration rules
//!    to produce a [`ModelConfig`] structure consumed by higher layers.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Minimal semantic view of a TLC model configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfig {
    pub init_operators: Vec<String>,
    pub next_operator: Option<String>,
    pub invariants: Vec<String>,
    pub properties: Vec<String>,
    pub specification: Option<String>,
    pub constraints: Vec<String>,
}

impl ModelConfig {
    pub fn defaults() -> Self {
        Self {
            init_operators: vec!["Init".to_string()],
            next_operator: Some("Next".to_string()),
            invariants: Vec::new(),
            properties: Vec::new(),
            specification: None,
            constraints: Vec::new(),
        }
    }
}

/// Errors surfaced while parsing configuration files.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read configuration '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid configuration directive '{directive}' on line {line}")]
    InvalidDirective { directive: String, line: usize },
}

/// Parse a TLC model configuration from disk.
pub fn parse_model_config(path: &Path) -> Result<ModelConfig, ConfigError> {
    let source = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_model_config_str(&source)
}

/// Parse a TLC model configuration from a string.
pub fn parse_model_config_str(source: &str) -> Result<ModelConfig, ConfigError> {
    let lines = lex_lines(source)?;
    Ok(parse_lines(&lines))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigDirective {
    Init,
    Inits,
    Next,
    Invariant,
    Invariants,
    Property,
    Properties,
    Constraint,
    Constraints,
    Specification,
}

impl ConfigDirective {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "INIT" => Some(Self::Init),
            "INITS" | "INITI" => Some(Self::Inits),
            "NEXT" => Some(Self::Next),
            "INVARIANT" => Some(Self::Invariant),
            "INVARIANTS" => Some(Self::Invariants),
            "PROPERTY" => Some(Self::Property),
            "PROPERTIES" => Some(Self::Properties),
            "CONSTRAINT" => Some(Self::Constraint),
            "CONSTRAINTS" => Some(Self::Constraints),
            "SPECIFICATION" => Some(Self::Specification),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ConfigLine {
    kind: LineKind,
}

#[derive(Debug, Clone)]
enum LineKind {
    Blank,
    Directive {
        keyword: ConfigDirective,
        body: String,
    },
    Continuation(String),
}

fn lex_lines(source: &str) -> Result<Vec<ConfigLine>, ConfigError> {
    let mut lines = Vec::new();

    for (index, raw) in source.lines().enumerate() {
        let line_number = index + 1;
        let stripped = strip_comment(raw);

        let indent = stripped
            .chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .count();
        let trimmed = stripped[indent..].trim_end();

        if trimmed.is_empty() {
            lines.push(ConfigLine {
                kind: LineKind::Blank,
            });
            continue;
        }

        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let head = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default().trim_start();
        let uppercase = head.to_ascii_uppercase();

        if let Some(keyword) = ConfigDirective::from_str(&uppercase) {
            lines.push(ConfigLine {
                kind: LineKind::Directive {
                    keyword,
                    body: rest.to_string(),
                },
            });
            continue;
        }

        if head.ends_with(':') {
            let directive = head.trim_end_matches(':').to_ascii_uppercase();
            if let Some(keyword) = ConfigDirective::from_str(&directive) {
                lines.push(ConfigLine {
                    kind: LineKind::Directive {
                        keyword,
                        body: rest.to_string(),
                    },
                });
                continue;
            }
        }

        if indent == 0 {
            return Err(ConfigError::InvalidDirective {
                directive: head.to_string(),
                line: line_number,
            });
        }

        lines.push(ConfigLine {
            kind: LineKind::Continuation(trimmed.to_string()),
        });
    }

    Ok(lines)
}

fn parse_lines(lines: &[ConfigLine]) -> ModelConfig {
    let mut config = ModelConfig::defaults();
    let mut current_block: Option<BlockKind> = None;
    let mut specification_buffer: Option<String> = None;

    for line in lines {
        match &line.kind {
            LineKind::Blank => continue,
            LineKind::Directive { keyword, body } => match keyword {
                ConfigDirective::Init | ConfigDirective::Inits => {
                    let names = parse_name_list(body);
                    if !names.is_empty() {
                        config.init_operators = names;
                    }
                    current_block = None;
                }
                ConfigDirective::Next => {
                    if body.is_empty() {
                        config.next_operator = None;
                    } else {
                        config.next_operator = Some(body.to_string());
                    }
                    current_block = None;
                }
                ConfigDirective::Invariant | ConfigDirective::Invariants => {
                    let names = parse_name_list(body);
                    config.invariants.extend(names);
                    current_block = Some(BlockKind::Invariant);
                }
                ConfigDirective::Property | ConfigDirective::Properties => {
                    let names = parse_name_list(body);
                    config.properties.extend(names);
                    current_block = Some(BlockKind::Property);
                }
                ConfigDirective::Constraint | ConfigDirective::Constraints => {
                    let names = parse_name_list(body);
                    config.constraints.extend(names);
                    current_block = Some(BlockKind::Constraint);
                }
                ConfigDirective::Specification => {
                    specification_buffer = if body.is_empty() {
                        Some(String::new())
                    } else {
                        Some(body.to_string())
                    };
                    current_block = Some(BlockKind::Specification);
                }
            },
            LineKind::Continuation(body) => {
                match current_block {
                    Some(BlockKind::Invariant) => {
                        let names = parse_name_list(body);
                        config.invariants.extend(names);
                    }
                    Some(BlockKind::Property) => {
                        let names = parse_name_list(body);
                        config.properties.extend(names);
                    }
                    Some(BlockKind::Constraint) => {
                        let names = parse_name_list(body);
                        config.constraints.extend(names);
                    }
                    Some(BlockKind::Specification) => {
                        let entry = specification_buffer.get_or_insert_with(String::new);
                        if !entry.is_empty() {
                            entry.push('\n');
                        }
                        entry.push_str(body.trim());
                    }
                    None => { /* ignore stray continuation lines */ }
                }
            }
        }
    }

    // Finalise specification buffer.
    if let Some(spec) = specification_buffer {
        if spec.trim().is_empty() {
            config.specification = None;
        } else {
            config.specification = Some(spec);
        }
    }

    config
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Invariant,
    Property,
    Constraint,
    Specification,
}

fn parse_name_list(input: &str) -> Vec<String> {
    input
        .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';')
        .filter(|token| !token.is_empty())
        .map(|token| token.trim().to_string())
        .collect()
}

fn strip_comment(line: &str) -> &str {
    if let Some(index) = line.find("\\*") {
        &line[..index]
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_init_and_next() {
        let parsed = parse_model_config_str("").expect("parse empty config");
        assert_eq!(parsed.init_operators, vec!["Init".to_string()]);
        assert_eq!(parsed.next_operator, Some("Next".to_string()));
        assert!(parsed.invariants.is_empty());
    }

    #[test]
    fn parses_init_next_and_invariants() {
        let source = r#"
        \* Sample TLC config
        INIT CustomInit
        NEXT Next_State
        INVARIANTS InvOne InvTwo
        \* Another comment
        "#;

        let parsed = parse_model_config_str(source).expect("parse config");
        assert_eq!(parsed.init_operators, vec!["CustomInit".to_string()]);
        assert_eq!(parsed.next_operator, Some("Next_State".to_string()));
        assert_eq!(
            parsed.invariants,
            vec!["InvOne".to_string(), "InvTwo".to_string()]
        );
    }

    #[test]
    fn handles_comma_separated_invariants() {
        let source = "INVARIANT InvA, InvB,InvC";
        let parsed = parse_model_config_str(source).expect("parse config");
        assert_eq!(
            parsed.invariants,
            vec!["InvA".to_string(), "InvB".to_string(), "InvC".to_string()]
        );
    }

    #[test]
    fn parses_indented_blocks() {
        let source = r#"
INVARIANTS
    InvA
    InvB
PROPERTIES
    PropA
CONSTRAINT FooConstraint
CONSTRAINTS
    BarConstraint
SPECIFICATION
    SpecInit /\ [SpecNext]_vars
"#;
        let parsed = parse_model_config_str(source).expect("parse config");
        assert_eq!(
            parsed.invariants,
            vec!["InvA".to_string(), "InvB".to_string()]
        );
        assert_eq!(parsed.properties, vec!["PropA".to_string()]);
        assert_eq!(
            parsed.constraints,
            vec!["FooConstraint".to_string(), "BarConstraint".to_string()]
        );
        assert_eq!(
            parsed.specification,
            Some("SpecInit /\\ [SpecNext]_vars".to_string())
        );
    }

    #[test]
    fn rejects_unknown_directive() {
        let err = parse_model_config_str("UNKNOWN Foo").unwrap_err();
        match err {
            ConfigError::InvalidDirective { directive, line } => {
                assert_eq!(directive, "UNKNOWN");
                assert_eq!(line, 1);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
