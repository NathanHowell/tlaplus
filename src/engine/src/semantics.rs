//! Semantic evaluation for TLA+ specifications.
//!
//! This module implements a lightweight evaluator that extracts the portions of a
//! specification required by the exploration engine: initial state generation,
//! successor computation, and invariant predicates. The evaluator intentionally
//! supports a constrained subset of the TLA+ grammar (equality, membership,
//! arithmetic on integers, boolean operators, and finite set literals) that
//! covers the bootstrap scenarios exercised by the early parity tasks. The
//! design keeps the representation extensible so future increments can add richer
//! expression handling without replacing the public API.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use tlc_util::parser::{Declaration, Module, OperatorDefinition};

/// Evaluated TLA+ value used by the semantic layer.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Set(Vec<Value>),
}

impl Value {
    fn as_int(&self) -> Result<i64, SemanticError> {
        match self {
            Value::Int(value) => Ok(*value),
            other => Err(SemanticError::TypeMismatch {
                expected: ValueKind::Int,
                found: other.kind(),
            }),
        }
    }

    fn as_bool(&self) -> Result<bool, SemanticError> {
        match self {
            Value::Bool(value) => Ok(*value),
            other => Err(SemanticError::TypeMismatch {
                expected: ValueKind::Bool,
                found: other.kind(),
            }),
        }
    }

    fn as_set(&self) -> Result<&[Value], SemanticError> {
        match self {
            Value::Set(values) => Ok(values),
            other => Err(SemanticError::TypeMismatch {
                expected: ValueKind::Set,
                found: other.kind(),
            }),
        }
    }

    fn kind(&self) -> ValueKind {
        match self {
            Value::Int(_) => ValueKind::Int,
            Value::Bool(_) => ValueKind::Bool,
            Value::Set(_) => ValueKind::Set,
        }
    }
}

/// High-level classification for [`Value`] used in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Int,
    Bool,
    Set,
}

/// Canonical TLC state representation used by the semantic evaluator.
pub type State = BTreeMap<String, Value>;

/// Inputs required to build [`ModelSemantics`].
#[derive(Debug, Clone)]
pub struct SemanticInputs<'a> {
    pub modules: &'a [Module],
    pub init_operators: Vec<String>,
    pub next_operator: String,
    pub invariant_operators: Vec<String>,
}

impl<'a> SemanticInputs<'a> {
    pub fn new(modules: &'a [Module]) -> Self {
        Self {
            modules,
            init_operators: Vec::new(),
            next_operator: String::new(),
            invariant_operators: Vec::new(),
        }
    }

    pub fn with_init_operators(mut self, names: &[impl AsRef<str>]) -> Self {
        self.init_operators = names.iter().map(|name| name.as_ref().to_string()).collect();
        self
    }

    pub fn with_next_operator(mut self, name: impl AsRef<str>) -> Self {
        self.next_operator = name.as_ref().to_string();
        self
    }

    pub fn with_invariant_operators(mut self, names: &[impl AsRef<str>]) -> Self {
        self.invariant_operators = names.iter().map(|name| name.as_ref().to_string()).collect();
        self
    }
}

/// Engine-facing semantics for a specification.
#[derive(Debug, Clone)]
pub struct ModelSemantics {
    variables: Vec<String>,
    initial_states: Vec<State>,
    next_constraints: Vec<NextConstraint>,
    invariants: Vec<Invariant>,
}

impl ModelSemantics {
    /// Build semantics from the provided inputs.
    pub fn new(inputs: SemanticInputs<'_>) -> Result<Self, SemanticError> {
        Builder::build(inputs)
    }

    /// All declared variables in declaration order.
    pub fn variables(&self) -> &[String] {
        &self.variables
    }

    /// Enumerated initial states derived from the `Init` operators.
    pub fn initial_states(&self) -> &[State] {
        &self.initial_states
    }

    /// Produce successor states for the provided state by evaluating the `Next` operator.
    pub fn successors(&self, state: &State) -> Result<Vec<State>, SemanticError> {
        evaluate_successors(state, &self.variables, &self.next_constraints)
    }

    /// Invariants declared in the semantic context.
    pub fn invariants(&self) -> &[Invariant] {
        &self.invariants
    }
}

/// Callable invariant predicate.
#[derive(Debug, Clone)]
pub struct Invariant {
    name: String,
    expr: Expr,
}

impl Invariant {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn evaluate(&self, state: &State) -> Result<bool, SemanticError> {
        let value = evaluate_expr(&self.expr, Some(state))?;
        value.as_bool()
    }
}

/// Errors surfaced by the semantic evaluator.
#[derive(Debug, Error)]
pub enum SemanticError {
    #[error("no init operators configured")]
    NoInitOperators,
    #[error("next operator must be configured")]
    MissingNextOperator,
    #[error("operator '{name}' not found in parsed modules")]
    OperatorNotFound { name: String },
    #[error("operator '{name}' arity mismatch: expected {expected}, found {found}")]
    OperatorArity {
        name: String,
        expected: usize,
        found: usize,
    },
    #[error("recursive operator invocation detected for '{name}'")]
    RecursiveOperator { name: String },
    #[error("unsupported init formula structure for variable '{variable}'")]
    UnsupportedInitClause { variable: String },
    #[error("init formula must assign every declared variable (missing '{variable}')")]
    MissingVariableAssignment { variable: String },
    #[error("init formula produced no values for variable '{variable}'")]
    EmptyVariableDomain { variable: String },
    #[error("next-state assignment must target a primed variable (got '{name}')")]
    InvalidNextVariable { name: String },
    #[error("expression evaluation failed: {message}")]
    Evaluation { message: String },
    #[error("type mismatch: expected {expected:?}, found {found:?}")]
    TypeMismatch {
        expected: ValueKind,
        found: ValueKind,
    },
}

impl SemanticError {
    fn evaluation(message: impl Into<String>) -> Self {
        SemanticError::Evaluation {
            message: message.into(),
        }
    }
}

/// Internal builder orchestrating semantic extraction.
struct Builder<'a> {
    inputs: SemanticInputs<'a>,
    operators: OperatorTable,
    variables: Vec<String>,
}

impl<'a> Builder<'a> {
    fn build(mut inputs: SemanticInputs<'a>) -> Result<ModelSemantics, SemanticError> {
        if inputs.init_operators.is_empty() {
            return Err(SemanticError::NoInitOperators);
        }
        let trimmed_next = inputs.next_operator.trim();
        if trimmed_next.is_empty() {
            return Err(SemanticError::MissingNextOperator);
        }
        inputs.next_operator = trimmed_next.to_string();

        let operators = OperatorTable::new(inputs.modules)?;
        let variables = collect_variables(inputs.modules);

        let mut builder = Builder {
            inputs,
            operators,
            variables,
        };

        builder.construct()
    }

    fn construct(&mut self) -> Result<ModelSemantics, SemanticError> {
        let init_exprs = self.load_init_exprs()?;
        let next_constraints = self.load_next_constraints()?;
        let invariants = self.load_invariants()?;
        let initial_states = enumerate_initial_states(&self.variables, &init_exprs)?;

        Ok(ModelSemantics {
            variables: self.variables.clone(),
            initial_states,
            next_constraints,
            invariants,
        })
    }

    fn load_init_exprs(&self) -> Result<Vec<Expr>, SemanticError> {
        self.inputs
            .init_operators
            .iter()
            .map(|name| self.operators.resolve_zero_arity(name))
            .collect()
    }

    fn load_next_constraints(&self) -> Result<Vec<NextConstraint>, SemanticError> {
        let expr = self
            .operators
            .resolve_zero_arity(&self.inputs.next_operator)?;
        extract_next_constraints(&expr)
    }

    fn load_invariants(&self) -> Result<Vec<Invariant>, SemanticError> {
        self.inputs
            .invariant_operators
            .iter()
            .map(|name| {
                let expr = self.operators.resolve_zero_arity(name)?;
                Ok(Invariant {
                    name: name.clone(),
                    expr,
                })
            })
            .collect()
    }
}

/// Constraint extracted from the next-state relation.
#[derive(Debug, Clone)]
enum NextConstraint {
    Assignment {
        variable: String,
        expr: Expr,
    },
    Choices {
        variable: String,
        options: Vec<Expr>,
    },
}

/// Operator definition table keyed by name.
#[derive(Debug, Clone)]
struct OperatorTable {
    defs: BTreeMap<String, OperatorEntry>,
}

#[derive(Debug, Clone)]
struct OperatorEntry {
    params: Vec<String>,
    body: Expr,
}

impl OperatorTable {
    fn new(modules: &[Module]) -> Result<Self, SemanticError> {
        let mut defs = BTreeMap::new();
        for module in modules {
            for decl in &module.declarations {
                if let Declaration::Operator(def) = decl {
                    let entry = OperatorEntry::from_definition(def)?;
                    defs.insert(def.name.name.clone(), entry);
                }
            }
        }
        Ok(Self { defs })
    }

    fn resolve_zero_arity(&self, name: &str) -> Result<Expr, SemanticError> {
        let entry = self
            .defs
            .get(name)
            .ok_or_else(|| SemanticError::OperatorNotFound {
                name: name.to_string(),
            })?;
        if !entry.params.is_empty() {
            return Err(SemanticError::OperatorArity {
                name: name.to_string(),
                expected: 0,
                found: entry.params.len(),
            });
        }

        let mut stack = Vec::new();
        expand_calls(&entry.body, self, &mut stack)
    }

    fn resolve_call(
        &self,
        name: &str,
        args: &[Expr],
        stack: &mut Vec<String>,
    ) -> Result<Expr, SemanticError> {
        if stack.contains(&name.to_string()) {
            return Err(SemanticError::RecursiveOperator {
                name: name.to_string(),
            });
        }

        let entry = self
            .defs
            .get(name)
            .ok_or_else(|| SemanticError::OperatorNotFound {
                name: name.to_string(),
            })?;

        if entry.params.len() != args.len() {
            return Err(SemanticError::OperatorArity {
                name: name.to_string(),
                expected: entry.params.len(),
                found: args.len(),
            });
        }

        stack.push(name.to_string());
        let mut substitutions = BTreeMap::new();
        for (param, arg) in entry.params.iter().zip(args.iter()) {
            let expanded = expand_calls(arg, self, stack)?;
            substitutions.insert(param.clone(), expanded);
        }
        let substituted = substitute_expr(&entry.body, &substitutions);
        let result = expand_calls(&substituted, self, stack)?;
        stack.pop();
        Ok(result)
    }
}

impl OperatorEntry {
    fn from_definition(def: &OperatorDefinition) -> Result<Self, SemanticError> {
        let params = def
            .params
            .iter()
            .map(|param| param.name.name.clone())
            .collect::<Vec<_>>();
        let body = parse_expression(&def.body.source)?;
        Ok(Self { params, body })
    }
}

/// Expressions supported by the semantic evaluator.
#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Number(i64),
    Bool(bool),
    Identifier(String),
    SetLiteral(Vec<Expr>),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    In,
    And,
    Or,
}

fn substitute_expr(expr: &Expr, replacements: &BTreeMap<String, Expr>) -> Expr {
    match expr {
        Expr::Identifier(name) => replacements
            .get(name)
            .cloned()
            .unwrap_or_else(|| expr.clone()),
        Expr::Unary { op, expr: inner } => Expr::Unary {
            op: *op,
            expr: Box::new(substitute_expr(inner, replacements)),
        },
        Expr::Binary { op, left, right } => Expr::Binary {
            op: *op,
            left: Box::new(substitute_expr(left, replacements)),
            right: Box::new(substitute_expr(right, replacements)),
        },
        Expr::SetLiteral(items) => Expr::SetLiteral(
            items
                .iter()
                .map(|item| substitute_expr(item, replacements))
                .collect(),
        ),
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_expr(arg, replacements))
                .collect(),
        },
        Expr::Number(_) | Expr::Bool(_) => expr.clone(),
    }
}

fn expand_calls(
    expr: &Expr,
    operators: &OperatorTable,
    stack: &mut Vec<String>,
) -> Result<Expr, SemanticError> {
    match expr {
        Expr::Call { name, args } => operators.resolve_call(name, args, stack),
        Expr::Unary { op, expr: inner } => Ok(Expr::Unary {
            op: *op,
            expr: Box::new(expand_calls(inner, operators, stack)?),
        }),
        Expr::Binary { op, left, right } => Ok(Expr::Binary {
            op: *op,
            left: Box::new(expand_calls(left, operators, stack)?),
            right: Box::new(expand_calls(right, operators, stack)?),
        }),
        Expr::SetLiteral(items) => Ok(Expr::SetLiteral(
            items
                .iter()
                .map(|item| expand_calls(item, operators, stack))
                .collect::<Result<_, _>>()?,
        )),
        Expr::Identifier(_) | Expr::Number(_) | Expr::Bool(_) => Ok(expr.clone()),
    }
}

fn collect_variables(modules: &[Module]) -> Vec<String> {
    let mut vars = BTreeSet::new();
    for module in modules {
        for decl in &module.declarations {
            if let Declaration::Variables { names, .. } = decl {
                for name in names {
                    vars.insert(name.name.clone());
                }
            }
        }
    }
    vars.into_iter().collect()
}

fn enumerate_initial_states(
    variables: &[String],
    init_exprs: &[Expr],
) -> Result<Vec<State>, SemanticError> {
    let mut domains: BTreeMap<String, Vec<Value>> = BTreeMap::new();

    for expr in init_exprs {
        collect_init_domains(expr, &mut domains)?;
    }

    for variable in variables {
        if !domains.contains_key(variable) {
            return Err(SemanticError::MissingVariableAssignment {
                variable: variable.clone(),
            });
        }
        if domains[variable].is_empty() {
            return Err(SemanticError::EmptyVariableDomain {
                variable: variable.clone(),
            });
        }
    }

    let ordered = variables
        .iter()
        .map(|name| (name.clone(), domains.remove(name).unwrap_or_default()))
        .collect::<Vec<_>>();

    Ok(cartesian_product(&ordered))
}

fn collect_init_domains(
    expr: &Expr,
    domains: &mut BTreeMap<String, Vec<Value>>,
) -> Result<(), SemanticError> {
    match expr {
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            collect_init_domains(left, domains)?;
            collect_init_domains(right, domains)?;
            Ok(())
        }
        Expr::Binary { op, left, right } if matches!(op, BinaryOp::Eq | BinaryOp::In) => {
            let variable = if let Expr::Identifier(name) = &**left {
                if name.ends_with('\'') {
                    return Err(SemanticError::UnsupportedInitClause {
                        variable: name.clone(),
                    });
                }
                name.clone()
            } else {
                return Err(SemanticError::UnsupportedInitClause {
                    variable: format!("{:?}", left),
                });
            };

            if matches!(op, BinaryOp::Eq) {
                let value = evaluate_expr(right, None)?;
                domains.entry(variable).or_default().push(value);
                return Ok(());
            }

            let set = evaluate_expr(right, None)?;
            let values = set.as_set()?.to_vec();
            domains.entry(variable).or_default().extend(values);
            Ok(())
        }
        _ => Err(SemanticError::UnsupportedInitClause {
            variable: format!("{expr:?}"),
        }),
    }
}

fn evaluate_successors(
    state: &State,
    variables: &[String],
    constraints: &[NextConstraint],
) -> Result<Vec<State>, SemanticError> {
    let mut domains: BTreeMap<String, Vec<Value>> = BTreeMap::new();

    for constraint in constraints {
        match constraint {
            NextConstraint::Assignment { variable, expr } => {
                let value = evaluate_expr(expr, Some(state))?;
                domains.entry(variable.clone()).or_default().push(value);
            }
            NextConstraint::Choices { variable, options } => {
                for option in options {
                    let value = evaluate_expr(option, Some(state))?;
                    domains.entry(variable.clone()).or_default().push(value);
                }
            }
        }
    }

    for variable in variables {
        if !domains.contains_key(variable) {
            if let Some(value) = state.get(variable) {
                domains
                    .entry(variable.clone())
                    .or_default()
                    .push(value.clone());
            } else {
                return Err(SemanticError::MissingVariableAssignment {
                    variable: variable.clone(),
                });
            }
        }
    }

    let ordered = variables
        .iter()
        .map(|name| (name.clone(), domains.remove(name).unwrap_or_default()))
        .collect::<Vec<_>>();

    Ok(cartesian_product_with_base(state, &ordered))
}

fn cartesian_product(domains: &[(String, Vec<Value>)]) -> Vec<State> {
    let mut states = Vec::new();
    fn helper(
        idx: usize,
        domains: &[(String, Vec<Value>)],
        current: &mut State,
        states: &mut Vec<State>,
    ) {
        if idx == domains.len() {
            states.push(current.clone());
            return;
        }

        let (name, values) = &domains[idx];
        for value in values {
            current.insert(name.clone(), value.clone());
            helper(idx + 1, domains, current, states);
        }
    }

    let mut current = State::new();
    helper(0, domains, &mut current, &mut states);
    states
}

fn cartesian_product_with_base(base: &State, domains: &[(String, Vec<Value>)]) -> Vec<State> {
    let mut states = Vec::new();
    fn helper(
        idx: usize,
        domains: &[(String, Vec<Value>)],
        current: &mut State,
        states: &mut Vec<State>,
    ) {
        if idx == domains.len() {
            states.push(current.clone());
            return;
        }

        let (name, values) = &domains[idx];
        for value in values {
            current.insert(name.clone(), value.clone());
            helper(idx + 1, domains, current, states);
        }
    }

    let mut current = base.clone();
    helper(0, domains, &mut current, &mut states);
    states
}

fn evaluate_expr(expr: &Expr, state: Option<&State>) -> Result<Value, SemanticError> {
    match expr {
        Expr::Number(value) => Ok(Value::Int(*value)),
        Expr::Bool(value) => Ok(Value::Bool(*value)),
        Expr::Identifier(name) => {
            if let Some(state) = state {
                state.get(name).cloned().ok_or_else(|| {
                    SemanticError::evaluation(format!("undefined identifier '{name}'"))
                })
            } else {
                Err(SemanticError::evaluation(format!(
                    "identifier '{name}' cannot be resolved without state"
                )))
            }
        }
        Expr::SetLiteral(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(evaluate_expr(item, state)?);
            }
            Ok(Value::Set(values))
        }
        Expr::Unary { op, expr: inner } => match op {
            UnaryOp::Not => {
                let value = evaluate_expr(inner, state)?;
                Ok(Value::Bool(!value.as_bool()?))
            }
            UnaryOp::Neg => {
                let value = evaluate_expr(inner, state)?;
                match value {
                    Value::Int(v) => Ok(Value::Int(-v)),
                    other => Err(SemanticError::TypeMismatch {
                        expected: ValueKind::Int,
                        found: other.kind(),
                    }),
                }
            }
        },
        Expr::Binary { op, left, right } => match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let lhs = evaluate_expr(left, state)?.as_int()?;
                let rhs = evaluate_expr(right, state)?.as_int()?;
                let result = match op {
                    BinaryOp::Add => lhs + rhs,
                    BinaryOp::Sub => lhs - rhs,
                    BinaryOp::Mul => lhs * rhs,
                    BinaryOp::Div => {
                        if rhs == 0 {
                            return Err(SemanticError::evaluation("division by zero"));
                        }
                        lhs / rhs
                    }
                    _ => unreachable!(),
                };
                Ok(Value::Int(result))
            }
            BinaryOp::Eq => {
                let lhs = evaluate_expr(left, state)?;
                let rhs = evaluate_expr(right, state)?;
                Ok(Value::Bool(lhs == rhs))
            }
            BinaryOp::In => {
                let lhs = evaluate_expr(left, state)?;
                let rhs = evaluate_expr(right, state)?;
                let set = rhs.as_set()?;
                Ok(Value::Bool(set.contains(&lhs)))
            }
            BinaryOp::And => {
                let lhs = evaluate_expr(left, state)?.as_bool()?;
                let rhs = evaluate_expr(right, state)?.as_bool()?;
                Ok(Value::Bool(lhs && rhs))
            }
            BinaryOp::Or => {
                let lhs = evaluate_expr(left, state)?.as_bool()?;
                let rhs = evaluate_expr(right, state)?.as_bool()?;
                Ok(Value::Bool(lhs || rhs))
            }
        },
        Expr::Call { name, .. } => Err(SemanticError::evaluation(format!(
            "unexpanded operator call '{name}'"
        ))),
    }
}

fn extract_next_constraints(expr: &Expr) -> Result<Vec<NextConstraint>, SemanticError> {
    let mut constraints = Vec::new();
    collect_next_constraints(expr, &mut constraints)?;
    Ok(constraints)
}

fn collect_next_constraints(
    expr: &Expr,
    constraints: &mut Vec<NextConstraint>,
) -> Result<(), SemanticError> {
    match expr {
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            collect_next_constraints(left, constraints)?;
            collect_next_constraints(right, constraints)?;
            Ok(())
        }
        Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } => {
            let variable = extract_next_variable(left)?;
            constraints.push(NextConstraint::Assignment {
                variable,
                expr: (**right).clone(),
            });
            Ok(())
        }
        Expr::Binary {
            op: BinaryOp::In,
            left,
            right,
        } => {
            let variable = extract_next_variable(left)?;
            if let Expr::SetLiteral(items) = &**right {
                constraints.push(NextConstraint::Choices {
                    variable,
                    options: items.clone(),
                });
                Ok(())
            } else {
                Err(SemanticError::UnsupportedInitClause {
                    variable: format!("{:?}", right),
                })
            }
        }
        _ => Err(SemanticError::evaluation(format!(
            "unsupported next-state clause: {expr:?}"
        ))),
    }
}

fn extract_next_variable(expr: &Expr) -> Result<String, SemanticError> {
    match expr {
        Expr::Identifier(name) => {
            if let Some(stripped) = name.strip_suffix('\'') {
                if stripped.is_empty() {
                    return Err(SemanticError::InvalidNextVariable { name: name.clone() });
                }
                Ok(stripped.to_string())
            } else {
                Err(SemanticError::InvalidNextVariable { name: name.clone() })
            }
        }
        _ => Err(SemanticError::InvalidNextVariable {
            name: format!("{expr:?}"),
        }),
    }
}

// Parser --------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Identifier(String),
    Number(i64),
    Bool(bool),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Operator(OperatorToken),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperatorToken {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    In,
    And,
    Or,
    Not,
}

struct Tokenizer<'a> {
    chars: std::str::Chars<'a>,
    peeked: Option<char>,
}

impl<'a> Tokenizer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars(),
            peeked: None,
        }
    }

    fn next_char(&mut self) -> Option<char> {
        if let Some(ch) = self.peeked.take() {
            Some(ch)
        } else {
            self.chars.next()
        }
    }

    fn peek_char(&mut self) -> Option<char> {
        if self.peeked.is_none() {
            self.peeked = self.chars.next();
        }
        self.peeked
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek_char(), Some(ch) if ch.is_whitespace()) {
            self.next_char();
        }
    }

    fn tokenize(mut self) -> Result<Vec<Token>, SemanticError> {
        let mut tokens = Vec::new();
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() {
                self.skip_whitespace();
                continue;
            }

            if ch.is_ascii_digit() {
                tokens.push(self.token_number()?);
                continue;
            }

            if is_identifier_start(ch) {
                tokens.push(self.token_identifier()?);
                continue;
            }

            tokens.push(self.token_symbol()?);
        }

        Ok(tokens)
    }

    fn token_number(&mut self) -> Result<Token, SemanticError> {
        let mut value = String::new();
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_digit() {
                value.push(ch);
                self.next_char();
            } else {
                break;
            }
        }

        let number = value
            .parse::<i64>()
            .map_err(|e| SemanticError::evaluation(format!("invalid number literal: {e}")))?;
        Ok(Token {
            kind: TokenKind::Number(number),
        })
    }

    fn token_identifier(&mut self) -> Result<Token, SemanticError> {
        let mut ident = String::new();
        while let Some(ch) = self.peek_char() {
            if is_identifier_continue(ch) {
                ident.push(ch);
                self.next_char();
            } else {
                break;
            }
        }

        let kind = match ident.as_str() {
            "TRUE" => TokenKind::Bool(true),
            "FALSE" => TokenKind::Bool(false),
            _ => TokenKind::Identifier(ident),
        };

        Ok(Token { kind })
    }

    fn token_symbol(&mut self) -> Result<Token, SemanticError> {
        let ch = self.next_char().expect("peeked char exists");
        let token = match ch {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            ',' => TokenKind::Comma,
            '+' => TokenKind::Operator(OperatorToken::Add),
            '-' => TokenKind::Operator(OperatorToken::Sub),
            '*' => TokenKind::Operator(OperatorToken::Mul),
            '/' => {
                if self.peek_char() == Some('\\') {
                    self.next_char();
                    TokenKind::Operator(OperatorToken::And)
                } else {
                    TokenKind::Operator(OperatorToken::Div)
                }
            }
            '\\' => match self.next_char() {
                Some('/') => TokenKind::Operator(OperatorToken::Or),
                Some('i') if self.next_char() == Some('n') => {
                    TokenKind::Operator(OperatorToken::In)
                }
                Some('*') => {
                    while let Some(ch) = self.next_char() {
                        if ch == '\n' {
                            break;
                        }
                    }
                    return self.token_symbol();
                }
                _ => {
                    return Err(SemanticError::evaluation(
                        "unsupported operator starting with backslash",
                    ))
                }
            },
            '=' => TokenKind::Operator(OperatorToken::Eq),
            '~' => TokenKind::Operator(OperatorToken::Not),
            _ => {
                return Err(SemanticError::evaluation(format!(
                    "unexpected character '{ch}'"
                )))
            }
        };

        Ok(Token { kind: token })
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit() || ch == '\''
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn parse_expression(&mut self) -> Result<Expr, SemanticError> {
        self.parse_binary_expression(0)
    }

    fn parse_binary_expression(&mut self, min_prec: u8) -> Result<Expr, SemanticError> {
        let mut left = self.parse_unary_expression()?;

        while let Some(op) = self.peek_operator() {
            let (precedence, right_assoc) = operator_precedence(op);
            if precedence < min_prec {
                break;
            }

            self.pos += 1;
            let next_min_prec = if right_assoc {
                precedence
            } else {
                precedence + 1
            };
            let right = self.parse_binary_expression(next_min_prec)?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_unary_expression(&mut self) -> Result<Expr, SemanticError> {
        if let Some(Token {
            kind: TokenKind::Operator(op),
        }) = self.tokens.get(self.pos)
        {
            let unary = match op {
                OperatorToken::Not => Some(UnaryOp::Not),
                OperatorToken::Sub => Some(UnaryOp::Neg),
                _ => None,
            };
            if let Some(operator) = unary {
                self.pos += 1;
                let expr = self.parse_unary_expression()?;
                return Ok(Expr::Unary {
                    op: operator,
                    expr: Box::new(expr),
                });
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, SemanticError> {
        match self.tokens.get(self.pos) {
            Some(Token {
                kind: TokenKind::Number(value),
            }) => {
                self.pos += 1;
                Ok(Expr::Number(*value))
            }
            Some(Token {
                kind: TokenKind::Bool(value),
            }) => {
                self.pos += 1;
                Ok(Expr::Bool(*value))
            }
            Some(Token {
                kind: TokenKind::Identifier(name),
            }) => {
                let ident = name.clone();
                self.pos += 1;

                if self.match_token(&TokenKind::LParen) {
                    let mut args = Vec::new();
                    if !self.match_token(&TokenKind::RParen) {
                        loop {
                            let expr = self.parse_expression()?;
                            args.push(expr);
                            if self.match_token(&TokenKind::RParen) {
                                break;
                            }
                            self.expect(TokenKind::Comma)?;
                        }
                    }
                    Ok(Expr::Call { name: ident, args })
                } else {
                    Ok(Expr::Identifier(ident))
                }
            }
            Some(Token {
                kind: TokenKind::LParen,
            }) => {
                self.pos += 1;
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            Some(Token {
                kind: TokenKind::LBrace,
            }) => {
                self.pos += 1;
                let mut items = Vec::new();
                if !self.match_token(&TokenKind::RBrace) {
                    loop {
                        let expr = self.parse_expression()?;
                        items.push(expr);
                        if self.match_token(&TokenKind::RBrace) {
                            break;
                        }
                        self.expect(TokenKind::Comma)?;
                    }
                }
                Ok(Expr::SetLiteral(items))
            }
            other => Err(SemanticError::evaluation(format!(
                "unexpected token {:?}",
                other
            ))),
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<(), SemanticError> {
        if self.match_token(&kind) {
            Ok(())
        } else {
            Err(SemanticError::evaluation(format!(
                "expected token {:?}",
                kind
            )))
        }
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if let Some(token) = self.tokens.get(self.pos) {
            if &token.kind == kind {
                self.pos += 1;
                return true;
            }
        }
        false
    }

    fn peek_operator(&self) -> Option<BinaryOp> {
        match self.tokens.get(self.pos) {
            Some(Token {
                kind: TokenKind::Operator(op),
            }) => match op {
                OperatorToken::Add => Some(BinaryOp::Add),
                OperatorToken::Sub => Some(BinaryOp::Sub),
                OperatorToken::Mul => Some(BinaryOp::Mul),
                OperatorToken::Div => Some(BinaryOp::Div),
                OperatorToken::Eq => Some(BinaryOp::Eq),
                OperatorToken::In => Some(BinaryOp::In),
                OperatorToken::And => Some(BinaryOp::And),
                OperatorToken::Or => Some(BinaryOp::Or),
                OperatorToken::Not => None,
            },
            _ => None,
        }
    }
}

fn operator_precedence(op: BinaryOp) -> (u8, bool) {
    match op {
        BinaryOp::Or => (1, false),
        BinaryOp::And => (2, false),
        BinaryOp::Eq | BinaryOp::In => (3, false),
        BinaryOp::Add | BinaryOp::Sub => (4, false),
        BinaryOp::Mul | BinaryOp::Div => (5, false),
    }
}

fn parse_expression(source: &str) -> Result<Expr, SemanticError> {
    let tokenizer = Tokenizer::new(source);
    let tokens = tokenizer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse_expression()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tlc_util::parser::{parse_module, ParserOptions};

    fn parse(text: &str) -> Module {
        parse_module(text, ParserOptions::default()).expect("module parses")
    }

    #[test]
    fn parses_simple_expression() {
        let expr = parse_expression("x + 1").expect("parse expression");
        assert!(matches!(
            expr,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn resolves_operator_calls() {
        let module = parse(
            r#"
---- MODULE Demo ----

Init == Setup(1)
Setup(val) == x = val

====
"#,
        );

        let table = OperatorTable::new(std::slice::from_ref(&module)).expect("build table");
        let expr = table.resolve_zero_arity("Init").expect("resolve");
        assert_eq!(
            expr,
            Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Identifier("x".into())),
                right: Box::new(Expr::Number(1)),
            }
        );
    }
}
