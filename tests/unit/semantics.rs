use std::collections::BTreeMap;

use tlc_engine::{ModelSemantics, SemanticInputs, State, Value};
use tlc_util::parser::{parse_module, ParserOptions};

fn parse_modules(source: &str) -> Vec<tlc_util::Module> {
    vec![parse_module(source, ParserOptions::default()).expect("module parses")]
}

fn build_inputs<'a>(modules: &'a [tlc_util::Module]) -> SemanticInputs<'a> {
    SemanticInputs::new(modules)
        .with_init_operators(&["Init"])
        .with_next_operator("Next")
        .with_invariant_operators(&["Inv"])
}

fn state(pairs: &[(&str, Value)]) -> State {
    let mut map = BTreeMap::new();
    for (name, value) in pairs {
        map.insert((*name).to_string(), value.clone());
    }
    map
}

#[test]
fn builds_initial_states_with_operator_substitution() {
    let modules = parse_modules(
        r#"
---- MODULE Demo ----
VARIABLE x, y

Init == Setup(2)
Setup(val) == /\ x = val /\ y = val + 1

Next == /\ x' = Advance(x)
        /\ y' \in Options(y, Advance(y))

Advance(delta) == delta + 1
Options(a, b) == { a, b }

Inv == y = x + 1

====
"#,
    );

    let semantics = ModelSemantics::new(build_inputs(&modules)).expect("semantics builds");

    let initial_states = semantics.initial_states();
    assert_eq!(initial_states.len(), 1);
    let expected = state(&[("x", Value::Int(2)), ("y", Value::Int(3))]);
    assert_eq!(initial_states[0], expected);

    let successors = semantics
        .successors(&initial_states[0])
        .expect("successors produced");
    // y can either stay at 3 or advance to 4, x always increments.
    assert_eq!(successors.len(), 2);
    assert!(successors.contains(&state(&[("x", Value::Int(3)), ("y", Value::Int(3))])));
    assert!(successors.contains(&state(&[("x", Value::Int(3)), ("y", Value::Int(4))])));

    let invariants = semantics.invariants();
    assert_eq!(invariants.len(), 1);
    assert!(invariants[0]
        .evaluate(&initial_states[0])
        .expect("evaluate invariant"));
    let results: Vec<bool> = successors
        .iter()
        .map(|state| {
            invariants[0]
                .evaluate(state)
                .expect("evaluate on successor")
        })
        .collect();
    assert!(results.contains(&false));
    assert!(results.contains(&true));
}

#[test]
fn enumerates_domains_from_membership_constraints() {
    let modules = parse_modules(
        r#"
---- MODULE DomainDemo ----
VARIABLE a, b

Init == /\ a \in {1, 2}
        /\ b = 5

Next == /\ a' \in {a, a + 1}
        /\ b' = b

Inv == b = 5

====
"#,
    );

    let semantics = ModelSemantics::new(build_inputs(&modules)).expect("semantics builds");
    let mut states = semantics.initial_states().to_vec();
    states.sort_by(|lhs, rhs| {
        let left = match lhs.get("a") {
            Some(Value::Int(v)) => *v,
            _ => 0,
        };
        let right = match rhs.get("a") {
            Some(Value::Int(v)) => *v,
            _ => 0,
        };
        left.cmp(&right)
    });
    assert_eq!(states.len(), 2);
    assert_eq!(
        states[0],
        state(&[("a", Value::Int(1)), ("b", Value::Int(5))])
    );
    assert_eq!(
        states[1],
        state(&[("a", Value::Int(2)), ("b", Value::Int(5))])
    );

    let successors = semantics
        .successors(&states[0])
        .expect("successors produced");
    assert!(successors
        .iter()
        .any(|s| s.get("a") == Some(&Value::Int(1))));
    assert!(successors
        .iter()
        .any(|s| s.get("a") == Some(&Value::Int(2))));
}
