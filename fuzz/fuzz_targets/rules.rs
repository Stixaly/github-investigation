//! Keyword rules: any analyst list is parsed without panicking, and any GitHub
//! text is evaluated with every hit pointing to a slice of the original text.
#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use observatory_domain::rules::Definition;
use serde_json::{Map, Value, json};

fn list(strip_accents: bool) -> Definition {
    let terms = json!(["é", "ﬁ", "ß", "İ", "a", "rat", "c2", "malware"]);
    let fields = json!(["repository.name", "repository.description", "repository.topics"]);
    Definition::parse(&json!({
        "format": "observatory.keyword-list", "schema_version": 1, "name": "fuzz",
        "normalization": {"strip_accents": strip_accents, "case_sensitive": false},
        "rules": [
            {"id": "words", "name": "Mots", "any": [{"fields": fields, "match": "whole_word", "terms": terms}]},
            {"id": "parts", "name": "Sous-chaînes", "any": [{"fields": fields, "match": "substring", "terms": terms}]},
            {"id": "phrase", "name": "Expression", "any": [{"fields": fields, "match": "exact_phrase",
                "terms": ["command and control", "a b"]}],
             "none": [{"fields": ["repository.readme"], "match": "substring", "terms": ["x"]}]}
        ]
    }))
    .expect("fuzz list is valid")
}

/// Built once: parsing them is not what this target measures.
static LISTS: LazyLock<[Definition; 2]> = LazyLock::new(|| [list(false), list(true)]);

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = serde_json::from_slice::<Value>(data) {
        if let Ok(definition) = Definition::parse(&value) {
            let _ = definition.failing_tests();
        }
    }
    let text = String::from_utf8_lossy(data).into_owned();
    let name: String = text.chars().take(64).collect();
    let topics: Vec<Value> = text.split(',').map(|topic| Value::String(topic.to_owned())).collect();
    let mut input = Map::new();
    input.insert("repository.name".into(), Value::String(name));
    input.insert("repository.description".into(), Value::String(text.clone()));
    input.insert("repository.topics".into(), Value::Array(topics));
    for definition in LISTS.iter() {
        for result in definition.evaluate(&input) {
            for hit in result.hits.iter().chain(&result.exclusions) {
                let original = match (&input[&hit.field], hit.index) {
                    (Value::Array(items), Some(index)) => items[index].as_str().expect("topic"),
                    (value, _) => value.as_str().expect("text field"),
                };
                assert!(hit.start < hit.end && hit.end <= original.len(), "{hit:?}");
                assert!(original.is_char_boundary(hit.start) && original.is_char_boundary(hit.end));
                assert!(hit.excerpt.contains(&original[hit.start..hit.end]));
            }
        }
    }
});
