use serde_json::json;

use super::*;

fn list(rules: &Value) -> Value {
    json!({"format": FORMAT, "schema_version": 1, "name": "Essai", "rules": rules})
}

fn one_rule(clause: &Value) -> Definition {
    Definition::parse(&list(&json!([{"id": "r", "name": "Règle", "any": [clause]}]))).unwrap()
}

fn input(value: &Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn outcome(definition: &Definition, value: &Value) -> Outcome {
    definition.evaluate(&input(value))[0].outcome
}

#[test]
fn the_roadmap_example_is_importable_and_its_tests_pass() {
    let example: Value =
        serde_json::from_str(include_str!("../../../docs/examples/keyword-list.example.json")).unwrap();
    let definition = Definition::parse(&example).unwrap();
    assert_eq!(definition.rules.len(), 3);
    assert_eq!(
        (definition.version, definition.status.as_deref()),
        (None, None),
        "server-managed fields are dropped"
    );
    assert_eq!(definition.failing_tests(), []);
}

#[test]
fn whole_words_are_not_substrings() {
    let words = one_rule(&json!({"fields": ["repository.name"], "match": "whole_word", "terms": ["rat"]}));
    assert_eq!(outcome(&words, &json!({"repository.name": "strategy"})), Outcome::NoMatch);
    assert_eq!(outcome(&words, &json!({"repository.name": "my-rat_tool"})), Outcome::Match);
    assert_eq!(outcome(&words, &json!({"repository.name": "RAT"})), Outcome::Match);
    let substrings =
        one_rule(&json!({"fields": ["repository.name"], "match": "substring", "terms": ["rat"]}));
    assert_eq!(outcome(&substrings, &json!({"repository.name": "strategy"})), Outcome::Match);
}

#[test]
fn unicode_forms_compare_equal_and_accents_only_on_request() {
    let words = one_rule(
        &json!({"fields": ["repository.description"], "match": "whole_word", "terms": ["malware", "fichier"]}),
    );
    // Fullwidth letters and the "fi" ligature have compatibility decompositions.
    assert_eq!(outcome(&words, &json!({"repository.description": "ＭＡＬＷＡＲＥ kit"})), Outcome::Match);
    assert_eq!(outcome(&words, &json!({"repository.description": "un ﬁchier"})), Outcome::Match);
    let accented = one_rule(
        &json!({"fields": ["repository.description"], "match": "whole_word", "terms": ["sécurité"]}),
    );
    // Composed and decomposed accents are the same text.
    assert_eq!(
        outcome(&accented, &json!({"repository.description": "se\u{301}curite\u{301}"})),
        Outcome::Match
    );
    assert_eq!(outcome(&accented, &json!({"repository.description": "securite"})), Outcome::NoMatch);
    let mut stripped = list(&json!([{"id": "r", "name": "Règle",
        "any": [{"fields": ["repository.description"], "match": "whole_word", "terms": ["sécurité"]}]}]));
    stripped["normalization"] = json!({"strip_accents": true});
    let stripped = Definition::parse(&stripped).unwrap();
    assert_eq!(outcome(&stripped, &json!({"repository.description": "SECURITE"})), Outcome::Match);
}

#[test]
fn hits_point_into_the_original_text() {
    let phrase = one_rule(
        &json!({"fields": ["repository.description"], "match": "exact_phrase", "terms": ["command and control"]}),
    );
    let text = "Un outil de COMMAND   and\ncontrol écrit en Rust";
    let result = &phrase.evaluate(&input(&json!({"repository.description": text})))[0];
    assert_eq!(result.outcome, Outcome::Match);
    let hit = &result.hits[0];
    assert_eq!(&text[hit.start..hit.end], "COMMAND   and\ncontrol");
    assert_eq!(hit.excerpt, text);
    let long = format!("{} malware {}", "a".repeat(60), "b".repeat(60));
    let words =
        one_rule(&json!({"fields": ["repository.description"], "match": "whole_word", "terms": ["malware"]}));
    let hit = &words.evaluate(&input(&json!({"repository.description": long})))[0].hits[0];
    assert!(hit.excerpt.starts_with('…') && hit.excerpt.ends_with('…'));
    assert_eq!(hit.excerpt.chars().count(), 2 + 40 + "malware".len() + 40);
}

#[test]
fn missing_fields_are_not_evaluated_and_lists_are_read_item_by_item() {
    let rule =
        one_rule(&json!({"fields": ["repository.description", "repository.readme"], "match": "exact_phrase",
                               "terms": ["malware analysis"]}));
    assert_eq!(outcome(&rule, &json!({"repository.description": null})), Outcome::NotEvaluated);
    assert_eq!(
        outcome(&rule, &json!({"repository.description": null, "repository.readme": ""})),
        Outcome::NoMatch
    );
    let result = &rule.evaluate(&input(&json!({"repository.description": "x"})))[0];
    assert_eq!(result.not_evaluated, ["repository.readme"]);
    let topics =
        one_rule(&json!({"fields": ["repository.topics"], "match": "exact_phrase", "terms": ["red team"]}));
    assert_eq!(outcome(&topics, &json!({"repository.topics": ["red", "team"]})), Outcome::NoMatch);
    let hit = &topics.evaluate(&input(&json!({"repository.topics": ["c2", "red team"]})))[0].hits[0];
    assert_eq!((hit.index, hit.start, hit.end), (Some(1), 0, 8));
}

#[test]
fn clauses_combine_with_three_valued_logic() {
    let definition = Definition::parse(&list(&json!([{
        "id": "c2", "name": "C2 hors recherche",
        "all": [
            {"fields": ["repository.name"], "match": "whole_word", "terms": ["c2"]},
            {"fields": ["repository.description"], "match": "substring", "terms": ["implant"]}
        ],
        "none": [{"fields": ["repository.readme"], "match": "exact_phrase", "terms": ["for research"]}]
    }])))
    .unwrap();
    let base = json!({"repository.name": "tiny-c2", "repository.description": "an implant"});
    // The exclusion cannot be checked without the README.
    assert_eq!(outcome(&definition, &base), Outcome::NotEvaluated);
    let mut with_readme = base.clone();
    with_readme["repository.readme"] = json!("Built for research only");
    let excluded = &definition.evaluate(&input(&with_readme))[0];
    assert_eq!((excluded.outcome, excluded.exclusions.len()), (Outcome::NoMatch, 1));
    let mut clean = base;
    clean["repository.readme"] = json!("usage");
    assert_eq!(outcome(&definition, &clean), Outcome::Match);
    // A known failing clause settles "all", whatever the unknown ones.
    assert_eq!(outcome(&definition, &json!({"repository.name": "web"})), Outcome::NoMatch);
}

#[test]
fn invalid_lists_are_explained() {
    let error = |rules: Value| Definition::parse(&list(&rules)).unwrap_err().to_string();
    assert!(error(json!([])).contains("1 à 50 règles"));
    assert!(error(json!([{"id": "r", "name": "x", "none": [{"fields": ["repository.name"], "match": "substring", "terms": ["x"]}]}]))
        .contains("clause positive"));
    assert!(error(json!([{"id": "r", "name": "x", "any": [{"fields": ["repository.stars"], "match": "substring", "terms": ["x"]}]}]))
        .contains("champ inconnu"));
    assert!(error(json!([{"id": "r", "name": "x", "any": [{"fields": ["repository.name"], "match": "whole_word", "terms": ["a b"]}]}]))
        .contains("exact_phrase"));
    let twice = json!({"id": "r", "name": "x", "any": [{"fields": ["repository.name"], "match": "substring", "terms": ["x"]}]});
    assert!(error(json!([twice.clone(), twice])).contains("en double"));
    assert!(error(json!([{"id": "R!", "name": "x", "any": []}])).contains("identifiant"));
    let mut forbidden = list(
        &json!([{"id": "r", "name": "x", "any": [{"fields": ["repository.name"], "match": "substring", "terms": ["x"]}]}]),
    );
    forbidden["execution"] = json!({"automatic_malicious_classification_enabled": true});
    assert!(Definition::parse(&forbidden).unwrap_err().to_string().contains("jamais activable"));
    let mut wrong = forbidden.clone();
    wrong["format"] = json!("observatory.keyword-list.proposal");
    assert!(Definition::parse(&wrong).is_err());
    let mut typo = list(&json!([]));
    typo["rulez"] = json!([]);
    assert!(Definition::parse(&typo).is_err(), "unknown keys are refused");
}

#[test]
fn failing_tests_are_reported() {
    let definition = Definition::parse(&list(&json!([{
        "id": "r", "name": "x",
        "any": [{"fields": ["repository.name"], "match": "whole_word", "terms": ["rat"]}],
        "tests": [{"input": {"repository.name": "strategy"}, "expected": "match"},
                  {"input": {"repository.name": "rat"}, "expected": "match"}]
    }])))
    .unwrap();
    assert_eq!(
        definition.failing_tests(),
        [TestFailure { rule: "r".into(), test: 0, expected: Outcome::Match, actual: Outcome::NoMatch }]
    );
    let bad_input = list(&json!([{"id": "r", "name": "x",
        "any": [{"fields": ["repository.name"], "match": "substring", "terms": ["x"]}],
        "tests": [{"input": {"repository.name": 3}, "expected": "match"}]}]));
    assert!(Definition::parse(&bad_input).is_err());
}

#[test]
fn disabled_rules_are_skipped() {
    let definition = Definition::parse(&list(&json!([
        {"id": "off", "name": "x", "enabled": false, "any": [{"fields": ["repository.name"], "match": "substring", "terms": ["x"]}]},
        {"id": "on", "name": "y", "any": [{"fields": ["repository.name"], "match": "substring", "terms": ["y"]}]}
    ])))
    .unwrap();
    let results = definition.evaluate(&input(&json!({"repository.name": "xy"})));
    assert_eq!(results.iter().map(|result| result.rule.as_str()).collect::<Vec<_>>(), ["on"]);
}
