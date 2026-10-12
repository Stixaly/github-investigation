use serde_json::{Value, json};

use super::*;

const REPOS: Options =
    Options { kind: SearchKind::Repositories, include_forks: true, max_pages: 2, max_subdivisions: 4 };
const CODE: Options =
    Options { kind: SearchKind::Code, include_forks: false, max_pages: 1, max_subdivisions: 2 };

fn list(rules: &Value) -> Definition {
    Definition::parse(
        &json!({"format": "observatory.keyword-list", "schema_version": 1, "name": "Liste", "rules": rules}),
    )
    .unwrap()
}

fn reference() -> ListRef {
    ListRef { id: "l1".into(), version: 3, name: "Thèmes".into() }
}

#[test]
fn dates_are_civil_days() {
    assert_eq!(date(0), "1970-01-01");
    assert_eq!(date(FIRST_DAY), "2008-01-01");
    assert_eq!(date(20_738), "2026-10-12");
    assert_eq!(date(11_016), "2000-02-29");
    assert_eq!(date(-1), "1969-12-31");
}

#[test]
fn queries_are_encoded() {
    assert_eq!(encode("\"VirtualAllocEx\" language:C++"), "%22VirtualAllocEx%22%20language%3AC%2B%2B");
    assert_eq!(
        search_url("https://api.github.com", SearchKind::Code, "a b"),
        "https://api.github.com/search/code?q=a%20b&per_page=100"
    );
}

#[test]
fn a_typed_query_shows_the_forks_choice_and_github_limits() {
    let plan = from_query("  loader   language:Rust ", REPOS).unwrap();
    assert_eq!(plan.queries[0].q, "loader language:Rust fork:true");
    assert_eq!(plan.label(), "Dépôts : loader language:Rust fork:true");
    assert_eq!(plan.max_requests(), (1 + 2 * 4) * 2);
    assert!(plan.warnings[0].contains("1000 premiers résultats"));
    let explicit = from_query("loader fork:false", REPOS).unwrap();
    assert_eq!(explicit.queries[0].q, "loader fork:false", "an explicit fork choice is kept");
    let code = from_query("\"VirtualAllocEx\" language:C", CODE).unwrap();
    assert_eq!(code.queries[0].q, "\"VirtualAllocEx\" language:C");
    assert!(code.warnings.iter().any(|warning| warning.contains("token GitHub requis")));
    assert!(code.warnings.iter().any(|warning| warning.contains("REM-30")));
}

#[test]
fn typed_queries_respect_github_limits() {
    assert!(from_query("", REPOS).is_err());
    assert!(from_query(&"a".repeat(260), REPOS).is_err());
    assert!(from_query("a OR b OR c OR d OR e OR f OR g", REPOS).unwrap_err().contains("opérateurs"));
    assert!(from_query("a\u{0}b", REPOS).is_err());
    assert!(from_query("a", Options { max_pages: 11, ..REPOS }).is_err());
    assert!(from_query("a", Options { max_subdivisions: 9, ..REPOS }).is_err());
}

#[test]
fn a_list_gives_one_query_per_term_and_says_what_stays_local() {
    let definition = list(&json!([
        {"id": "themes", "name": "Thèmes",
         "any": [{"fields": ["repository.name", "repository.full_name", "repository.description", "account.login"],
                  "match": "whole_word", "terms": ["stealer"]},
                 {"fields": ["repository.name", "repository.description"],
                  "match": "exact_phrase", "terms": ["command and control"]}],
         "all": [{"fields": ["repository.readme"], "match": "exact_phrase", "terms": ["keylogger"]}],
         "none": [{"fields": ["repository.description"], "match": "substring", "terms": ["research"]}]},
        {"id": "parts", "name": "Parties", "any": [{"fields": ["repository.topics"], "match": "substring", "terms": ["rat"]}]},
        {"id": "off", "name": "Désactivée", "enabled": false,
         "any": [{"fields": ["repository.name"], "match": "whole_word", "terms": ["ignored"]}]}
    ]));
    let plan = from_list(&definition, reference(), REPOS).unwrap();
    let queries: Vec<&str> = plan.queries.iter().map(|query| query.q.as_str()).collect();
    assert_eq!(
        queries,
        [
            "\"stealer\" in:name,description fork:true",
            "\"command and control\" in:name,description fork:true",
            "\"keylogger\" in:readme fork:true",
            "\"rat\" in:topics fork:true",
        ]
    );
    assert_eq!(plan.queries[0].origin, "themes : stealer");
    assert_eq!(plan.label(), "Dépôts : liste « Thèmes » v3");
    assert_eq!(plan.local_checks.len(), 2, "all and none are checked locally");
    assert!(plan.warnings.iter().any(|warning| warning.contains("account.login non cherchables")));
    assert!(plan.warnings.iter().any(|warning| warning.contains("plus restrictive")));
}

#[test]
fn lists_are_bounded_and_searched_in_repositories_only() {
    let terms: Vec<String> = (0..21).map(|index| format!("term{index}")).collect();
    let many = list(
        &json!([{"id": "r", "name": "x", "any": [{"fields": ["repository.name"], "match": "whole_word", "terms": terms}]}]),
    );
    assert!(from_list(&many, reference(), REPOS).unwrap_err().contains("20 au plus"));
    assert!(from_list(&many, reference(), CODE).is_err());
    let accounts = list(
        &json!([{"id": "r", "name": "x", "any": [{"fields": ["account.login"], "match": "whole_word", "terms": ["x"]}]}]),
    );
    assert!(from_list(&accounts, reference(), REPOS).unwrap_err().contains("Aucun terme"));
}

#[test]
fn saturated_searches_split_in_disjoint_halves() {
    let plan = from_query("loader", REPOS).unwrap();
    let today = 20_738;
    let [first, second] = plan.queries[0].subdivide(SearchKind::Repositories, today).unwrap();
    assert_eq!(first.q, "loader fork:true created:2008-01-01..2017-05-22");
    assert_eq!(second.q, "loader fork:true created:2017-05-23..2026-10-12");
    let [deeper, _] = second.subdivide(SearchKind::Repositories, today).unwrap();
    assert!(deeper.q.ends_with("created:2017-05-23..2022-01-31"));
    let single = Query { slice: Some(Slice::Created { from: 100, to: 100 }), ..first.clone() };
    assert!(single.subdivide(SearchKind::Repositories, today).is_none(), "one day cannot be split");
    let dated = from_query("loader created:>2024-01-01", REPOS).unwrap();
    assert!(dated.queries[0].subdivide(SearchKind::Repositories, today).is_none());
    let code = from_query("VirtualAllocEx", CODE).unwrap();
    let [small, large] = code.queries[0].subdivide(SearchKind::Code, today).unwrap();
    assert_eq!(
        (small.q.as_str(), large.q.as_str()),
        ("VirtualAllocEx size:0..196608", "VirtualAllocEx size:196609..393216")
    );
    // Plans read back as stored.
    let stored = serde_json::to_value(&plan).unwrap();
    assert_eq!(serde_json::from_value::<Plan>(stored).unwrap(), plan);
}
