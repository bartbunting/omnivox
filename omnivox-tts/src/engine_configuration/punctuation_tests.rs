use super::*;
use serde_json::json;

fn parse(punctuation: Value) -> Result<Configuration> {
    Configuration::parse(
        &serde_json::to_vec(&json!({"schema":3,"speech":{"punctuation":punctuation}})).unwrap(),
        Platform::native(),
    )
}

#[test]
fn sparse_overrides_preserve_other_levels_and_defaults() {
    let config = parse(json!({"some":{"’":"single quote","$":null},"all":{"'":"tick"}})).unwrap();
    let tables = config.speech.punctuation;
    assert_eq!(
        tables.spoken_name('’', PunctuationLevel::Some),
        Some("single quote")
    );
    assert_eq!(
        tables.spoken_name('’', PunctuationLevel::All),
        Some("apostrophe")
    );
    assert_eq!(tables.spoken_name('$', PunctuationLevel::Some), None);
    assert_eq!(
        tables.spoken_name('$', PunctuationLevel::None),
        Some("dollar")
    );
    assert_eq!(
        tables.spoken_name('\'', PunctuationLevel::All),
        Some("tick")
    );
    assert_eq!(
        parse(json!({})).unwrap().speech.punctuation,
        PunctuationTables::default()
    );
}

#[test]
fn punctuation_rejects_unknown_levels_invalid_scalars_names_and_limits() {
    for value in [
        json!(null),
        json!([]),
        json!({"custom":{}}),
        json!({"some":null}),
        json!({"some":{"":"name"}}),
        json!({"some":{"ab":"name"}}),
        json!({"some":{" ":"space"}}),
        json!({"some":{"\n":"newline"}}),
        json!({"some":{"’":true}}),
        json!({"some":{"’":{}}}),
        json!({"some":{"’":""}}),
        json!({"some":{"’":" "}}),
        json!({"some":{"’":" edge"}}),
        json!({"some":{"’":"edge "}}),
        json!({"some":{"’":"bad\nname"}}),
        json!({"some":{"’":"é".repeat(33)}}),
    ] {
        assert!(parse(value).is_err());
    }
    assert!(parse(json!({"some":{"’":"é".repeat(32)}})).is_ok());
    let mut table = PunctuationTable::new();
    for point in 0x1000..(0x1000 + 512 - PunctuationTables::default().some.len() as u32) {
        table.insert(char::from_u32(point).unwrap(), None);
    }
    assert!(parse(json!({"some":table})).is_ok());
    table.insert('※', None);
    assert!(parse(json!({"some":table})).is_err());
    assert!(Configuration::parse(
        br#"{"schema":3,"speech":{"punctuation":{"some":{"$":null}},"defaults":{"voice":null}}}"#,
        Platform::native(),
    )
    .is_err());
    for schema in [1, 2] {
        assert!(Configuration::parse(
            &serde_json::to_vec(&json!({"schema":schema,"speech":{"punctuation":{}}})).unwrap(),
            Platform::native(),
        )
        .is_err());
    }
    // Duplicate detection works after JSON escape decoding, before map collection.
    for input in [
        r#"{"schema":3,"speech":{"punctuation":{"some":{"'":"one","\u0027":"two"}}}}"#,
        r#"{"schema":3,"speech":{"punctuation":{"some":{},"some":{}}}}"#,
    ] {
        assert!(Configuration::parse(input.as_bytes(), Platform::native()).is_err());
    }
}

#[test]
fn all_shipped_tables_are_valid_and_ascii_is_unchanged() {
    let tables = PunctuationTables::default();
    tables.validate().unwrap();
    let legacy = PunctuationTables::legacy();
    for level in [
        PunctuationLevel::None,
        PunctuationLevel::Some,
        PunctuationLevel::All,
    ] {
        for c in '\0'..='\x7f' {
            assert_eq!(tables.spoken_name(c, level), legacy.spoken_name(c, level));
        }
    }
}

#[test]
fn older_public_configurations_get_apostrophe_defaults_and_explicit_preservation_wins() {
    for schema in 1..=3 {
        let configuration = Configuration::parse(
            &serde_json::to_vec(&json!({"schema":schema})).unwrap(),
            Platform::native(),
        )
        .unwrap();
        for character in ['\'', '‘', '’', 'ʼ'] {
            assert_eq!(
                configuration
                    .speech
                    .punctuation
                    .spoken_name(character, PunctuationLevel::Some),
                None
            );
            assert_eq!(
                configuration
                    .speech
                    .punctuation
                    .spoken_name(character, PunctuationLevel::All),
                Some("apostrophe")
            );
        }
    }
    let configuration = parse(json!({"all":{"'":null,"‘":null,"’":null,"ʼ":null}})).unwrap();
    for character in ['\'', '‘', '’', 'ʼ'] {
        assert_eq!(
            configuration
                .speech
                .punctuation
                .spoken_name(character, PunctuationLevel::Some),
            None
        );
        assert_eq!(
            configuration
                .speech
                .punctuation
                .spoken_name(character, PunctuationLevel::All),
            None
        );
    }
}
