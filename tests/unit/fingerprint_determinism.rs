use serde::Deserialize;
use tlc_util::{FingerprintBuilder, StateFingerprint};

#[derive(Debug, Deserialize)]
struct Fixture {
    label: String,
    generation: u64,
    #[serde(default)]
    context: Option<String>,
    steps: Vec<Step>,
    expected_hex: String,
    checksum_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Step {
    Text { value: String },
    Bytes { value: Vec<u8> },
    U64 { value: u64 },
    U128 { value: String },
}

impl Step {
    fn apply<'a>(&'a self, builder: FingerprintBuilder) -> FingerprintBuilder {
        match self {
            Step::Text { value } => builder.update(value.as_bytes()),
            Step::Bytes { value } => builder.update(value),
            Step::U64 { value } => builder.update_u64(*value),
            Step::U128 { value } => {
                let parsed =
                    u128::from_str_radix(value, 16).expect("u128 fixture should be valid hex");
                builder.update_u128(parsed)
            }
        }
    }
}

#[test]
fn fingerprint_fixtures_stay_canonical() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("fixtures/fingerprint_fixtures.json"))
            .expect("fixtures should parse");

    for fixture in fixtures {
        let mut builder = match &fixture.context {
            Some(ctx) => FingerprintBuilder::with_context(ctx.as_bytes()),
            None => FingerprintBuilder::default(),
        };

        for step in &fixture.steps {
            builder = step.apply(builder);
        }

        let fingerprint = builder.finalize(fixture.generation);
        let expected_checksum =
            u32::from_str_radix(&fixture.checksum_hex, 16).expect("checksum should be hex");

        assert_eq!(
            fixture.expected_hex,
            fingerprint.to_hex(),
            "{} fingerprint changed; determinism regression?",
            fixture.label
        );
        assert_eq!(
            expected_checksum,
            fingerprint.checksum(),
            "{} checksum changed; determinism regression?",
            fixture.label
        );

        // Round trips should conserve the fingerprint value.
        let as_bytes = fingerprint.to_bytes();
        let parsed_from_bytes =
            StateFingerprint::from_bytes(&as_bytes, fixture.generation, fingerprint.checksum())
                .expect("round-trip from bytes");
        assert_eq!(
            fingerprint.value(),
            parsed_from_bytes.value(),
            "{} byte encoding mismatch",
            fixture.label
        );

        let parsed_from_hex = StateFingerprint::from_hex(
            &fixture.expected_hex,
            fixture.generation,
            expected_checksum,
        )
        .expect("round-trip from hex");
        assert_eq!(
            fingerprint.value(),
            parsed_from_hex.value(),
            "{} hex parsing mismatch",
            fixture.label
        );
    }
}

#[test]
fn single_chunk_matches_state_fingerprint_new() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("fixtures/fingerprint_fixtures.json"))
            .expect("fixtures should parse");

    for fixture in fixtures.iter().filter(|f| f.context.is_none()) {
        if fixture.steps.len() == 1 {
            match &fixture.steps[0] {
                Step::Text { value } => {
                    let via_builder = FingerprintBuilder::default()
                        .update(value.as_bytes())
                        .finalize(fixture.generation);
                    let via_new = StateFingerprint::new(value.as_bytes(), fixture.generation);
                    assert_eq!(
                        via_builder.value(),
                        via_new.value(),
                        "{} `StateFingerprint::new` mismatch",
                        fixture.label
                    );
                    assert_eq!(
                        via_builder.checksum(),
                        via_new.checksum(),
                        "{} `StateFingerprint::new` checksum mismatch",
                        fixture.label
                    );
                }
                Step::Bytes { value } => {
                    let via_builder = FingerprintBuilder::default()
                        .update(value)
                        .finalize(fixture.generation);
                    let via_new = StateFingerprint::new(value, fixture.generation);
                    assert_eq!(
                        via_builder.value(),
                        via_new.value(),
                        "{} `StateFingerprint::new` mismatch",
                        fixture.label
                    );
                    assert_eq!(
                        via_builder.checksum(),
                        via_new.checksum(),
                        "{} `StateFingerprint::new` checksum mismatch",
                        fixture.label
                    );
                }
                _ => {}
            }
        }
    }
}
