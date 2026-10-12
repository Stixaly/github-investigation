//! Listes de termes : contrat JSON versionné, normalisation et moteur à trois états.
//!
//! Le format suit la feuille de route (§4.3 à 4.5) et
//! `docs/examples/keyword-list.example.json`. Une règle retourne `match`,
//! `no_match` ou `not_evaluated` : un champ absent de l'observation n'est jamais
//! traité comme vide. Une correspondance est une priorité de revue, jamais une
//! qualification du dépôt ou de son auteur.
//!
//! Normalisation, version 1 : décomposition de compatibilité Unicode caractère
//! par caractère, minuscules, espaces regroupés, accents retirés seulement sur
//! demande. Deux textes ont la même forme NFKC si et seulement s'ils ont la même
//! forme NFKD : la décomposition suffit pour comparer, et chaque octet normalisé
//! garde la position du caractère d'origine pour l'extrait.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unicode_normalization::char::{decompose_compatible, is_combining_mark};

/// Valeur du champ `format` d'une liste importable.
pub const FORMAT: &str = "observatory.keyword-list";
pub const SCHEMA_VERSION: u64 = 1;
/// Version du moteur et de la normalisation, conservée avec chaque évaluation.
pub const ENGINE_VERSION: u32 = 1;

/// Champs évaluables. README et fichiers ne le sont qu'une fois acquis.
pub const FIELDS: &[&str] = &[
    "repository.name",
    "repository.full_name",
    "repository.description",
    "repository.topics",
    "repository.readme",
    "account.login",
    "account.name",
];

const MAX_RULES: usize = 50;
const MAX_CLAUSES: usize = 20;
const MAX_TERMS: usize = 1000;
const MAX_TERM_CHARS: usize = 200;
const MAX_TESTS: usize = 50;
/// Correspondances gardées au plus par clause, pour l'explication.
const MAX_HITS: usize = 5;
/// Caractères de contexte de part et d'autre d'un extrait.
const EXCERPT_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuleError {
    #[error("Liste invalide : {0}")]
    Format(String),
    #[error("Règle « {rule} » : {reason}")]
    Rule { rule: String, reason: String },
}

fn rule_error(rule: &str, reason: impl Into<String>) -> RuleError {
    RuleError::Rule { rule: rule.to_owned(), reason: reason.into() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Un mot isolé, entouré de frontières (début, fin, ou caractère non alphanumérique).
    WholeWord,
    /// Une expression de plusieurs mots, dans un même champ et entourée de frontières.
    ExactPhrase,
    /// Une sous-chaîne, choix explicite : `rat` trouve alors `strategy`.
    Substring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Informational,
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Match,
    NoMatch,
    NotEvaluated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clause {
    pub fields: Vec<String>,
    #[serde(rename = "match")]
    pub mode: MatchMode,
    pub terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleTest {
    pub input: Map<String, Value>,
    pub expected: Outcome,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub review_priority: Priority,
    #[serde(default)]
    pub any: Vec<Clause>,
    #[serde(default)]
    pub all: Vec<Clause>,
    #[serde(default)]
    pub none: Vec<Clause>,
    #[serde(default)]
    pub tests: Vec<RuleTest>,
}

fn nfkc() -> String {
    "NFKC".to_owned()
}

fn not_evaluated() -> String {
    "not_evaluated".to_owned()
}

fn separators() -> Vec<String> {
    ["-", "_", ".", "/"].iter().map(|separator| (*separator).to_owned()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Normalization {
    #[serde(default = "nfkc")]
    pub unicode: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub strip_accents: bool,
    /// Frontières de mots des noms de dépôts. Tout caractère non alphanumérique en
    /// est déjà une : la liste documente le contrat et ne peut que l'étendre.
    #[serde(default = "separators")]
    pub repository_name_separators: Vec<String>,
    #[serde(default = "not_evaluated")]
    pub missing_field_result: String,
}

impl Default for Normalization {
    fn default() -> Self {
        Self {
            unicode: nfkc(),
            case_sensitive: false,
            strip_accents: false,
            repository_name_separators: separators(),
            missing_field_result: not_evaluated(),
        }
    }
}

fn investigation() -> String {
    "investigation".to_owned()
}

/// Liste de termes telle qu'importée, publiée et exportée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub format: String,
    pub schema_version: u64,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "investigation")]
    pub scope: String,
    #[serde(default)]
    pub normalization: Normalization,
    pub rules: Vec<Rule>,
    /// Préférences d'exécution ; l'application garde la décision de lancer une collecte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Map<String, Value>>,
    /// Attribués par la publication : ignorés à l'import, ajoutés à l'export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Échec d'un test intégré à une règle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestFailure {
    pub rule: String,
    pub test: usize,
    pub expected: Outcome,
    pub actual: Outcome,
}

impl Definition {
    /// Lit et valide une liste ; `version` et `status` sont retirés.
    pub fn parse(value: &Value) -> Result<Self, RuleError> {
        let mut definition: Self =
            serde_json::from_value(value.clone()).map_err(|error| RuleError::Format(error.to_string()))?;
        definition.version = None;
        definition.status = None;
        definition.validate()?;
        Ok(definition)
    }

    fn validate(&self) -> Result<(), RuleError> {
        let format = |reason: &str| Err(RuleError::Format(reason.to_owned()));
        if self.format != FORMAT {
            return Err(RuleError::Format(format!("format « {FORMAT} » attendu")));
        }
        if self.schema_version != SCHEMA_VERSION {
            return format("schema_version 1 attendu");
        }
        if !(1..=120).contains(&self.name.trim().chars().count()) || self.description.chars().count() > 2000 {
            return format("nom de 1 à 120 caractères, description de 2 000 au plus");
        }
        if self.scope != "investigation" {
            return format("seule la portée « investigation » est disponible");
        }
        let normalization = &self.normalization;
        if normalization.unicode != "NFKC" || normalization.missing_field_result != "not_evaluated" {
            return format("normalisation NFKC et champs absents « not_evaluated » seulement");
        }
        if normalization
            .repository_name_separators
            .iter()
            .any(|separator| separator.chars().count() != 1 || separator.chars().any(char::is_alphanumeric))
        {
            return format("séparateurs : un caractère non alphanumérique chacun");
        }
        if let Some(execution) = &self.execution {
            for forbidden in
                ["automatic_malicious_classification_enabled", "automatic_external_notification_enabled"]
            {
                if execution.get(forbidden) == Some(&Value::Bool(true)) {
                    return Err(RuleError::Format(format!("{forbidden} : jamais activable")));
                }
            }
        }
        if self.rules.is_empty() || self.rules.len() > MAX_RULES {
            return format("1 à 50 règles");
        }
        let mut ids = HashSet::new();
        let mut terms = 0;
        for rule in &self.rules {
            let valid_id = (1..=64).contains(&rule.id.len())
                && rule
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte));
            if !valid_id {
                return Err(rule_error(&rule.id, "identifiant de 1 à 64 caractères : a-z, 0-9, - et _"));
            }
            if !ids.insert(rule.id.as_str()) {
                return Err(rule_error(&rule.id, "identifiant en double"));
            }
            if !(1..=120).contains(&rule.name.trim().chars().count()) || rule.category.chars().count() > 60 {
                return Err(rule_error(&rule.id, "nom de 1 à 120 caractères, catégorie de 60 au plus"));
            }
            if rule.any.is_empty() && rule.all.is_empty() {
                return Err(rule_error(&rule.id, "au moins une clause positive (any ou all)"));
            }
            let clauses = rule.any.iter().chain(&rule.all).chain(&rule.none);
            if clauses.clone().count() > MAX_CLAUSES {
                return Err(rule_error(&rule.id, "20 clauses au plus"));
            }
            for clause in clauses {
                terms += self.validate_clause(&rule.id, clause)?;
            }
            if rule.tests.len() > MAX_TESTS {
                return Err(rule_error(&rule.id, "50 tests au plus"));
            }
            for test in &rule.tests {
                validate_input(&test.input).map_err(|reason| rule_error(&rule.id, reason))?;
            }
        }
        if terms > MAX_TERMS {
            return format("1 000 termes au plus pour toute la liste");
        }
        Ok(())
    }

    fn validate_clause(&self, rule: &str, clause: &Clause) -> Result<usize, RuleError> {
        if clause.fields.is_empty() || clause.terms.is_empty() {
            return Err(rule_error(rule, "chaque clause cite au moins un champ et un terme"));
        }
        if let Some(field) = clause.fields.iter().find(|field| !FIELDS.contains(&field.as_str())) {
            return Err(rule_error(rule, format!("champ inconnu « {field} »")));
        }
        for term in &clause.terms {
            let normalized = normalize(term, &self.normalization).text;
            let normalized = normalized.trim();
            if normalized.is_empty() || term.chars().count() > MAX_TERM_CHARS {
                return Err(rule_error(rule, "termes de 1 à 200 caractères, non vides après normalisation"));
            }
            if clause.mode == MatchMode::WholeWord && normalized.contains(' ') {
                return Err(rule_error(
                    rule,
                    format!("« {term} » contient un espace : utiliser exact_phrase"),
                ));
            }
        }
        Ok(clause.terms.len())
    }

    /// Exécute les tests intégrés aux règles ; une liste n'est publiée que s'ils réussissent tous.
    pub fn failing_tests(&self) -> Vec<TestFailure> {
        let mut failures = Vec::new();
        for rule in &self.rules {
            for (index, test) in rule.tests.iter().enumerate() {
                let actual = evaluate_rule(rule, &self.normalization, &test.input).outcome;
                if actual != test.expected {
                    failures.push(TestFailure {
                        rule: rule.id.clone(),
                        test: index,
                        expected: test.expected,
                        actual,
                    });
                }
            }
        }
        failures
    }

    /// Évalue les règles actives sur une observation.
    pub fn evaluate(&self, input: &Map<String, Value>) -> Vec<RuleResult> {
        self.rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| evaluate_rule(rule, &self.normalization, input))
            .collect()
    }
}

/// Une observation à évaluer : chaque champ est une chaîne, `null` (connu et vide)
/// ou une liste de chaînes. Un champ absent reste inconnu.
fn validate_input(input: &Map<String, Value>) -> Result<(), String> {
    for (field, value) in input {
        if !FIELDS.contains(&field.as_str()) {
            return Err(format!("champ de test inconnu « {field} »"));
        }
        let valid = match value {
            Value::Null | Value::String(_) => true,
            Value::Array(items) => items.iter().all(Value::is_string),
            _ => false,
        };
        if !valid {
            return Err(format!("« {field} » : chaîne, null ou liste de chaînes"));
        }
    }
    Ok(())
}

/// Occurrence d'un terme, avec sa position dans le texte d'origine (octets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub field: String,
    /// Rang de la valeur dans un champ liste (topics).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    pub term: String,
    pub start: usize,
    pub end: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleResult {
    pub rule: String,
    pub outcome: Outcome,
    pub review_priority: Priority,
    /// Occurrences des clauses positives (any, all) qui ont abouti.
    pub hits: Vec<Hit>,
    /// Occurrences des exclusions (none) qui ont abouti.
    pub exclusions: Vec<Hit>,
    /// Champs cités par la règle et absents de l'observation.
    pub not_evaluated: Vec<String>,
}

/// Logique à trois états de Kleene : `None` = inconnu.
type Truth = Option<bool>;

fn or(values: impl Iterator<Item = Truth>) -> Truth {
    let mut result = Some(false);
    for value in values {
        match value {
            Some(true) => return Some(true),
            None => result = None,
            Some(false) => {}
        }
    }
    result
}

fn and(values: impl Iterator<Item = Truth>) -> Truth {
    let mut result = Some(true);
    for value in values {
        match value {
            Some(false) => return Some(false),
            None => result = None,
            Some(true) => {}
        }
    }
    result
}

fn evaluate_rule(rule: &Rule, normalization: &Normalization, input: &Map<String, Value>) -> RuleResult {
    let mut missing: Vec<String> = Vec::new();
    let mut evaluate = |clause: &Clause, hits: &mut Vec<Hit>| -> Truth {
        let (truth, clause_hits, clause_missing) = evaluate_clause(clause, normalization, input);
        hits.extend(clause_hits);
        for field in clause_missing {
            if !missing.contains(&field) {
                missing.push(field);
            }
        }
        truth
    };
    let mut hits = Vec::new();
    let mut exclusions = Vec::new();
    let any = if rule.any.is_empty() {
        Some(true)
    } else {
        or(rule.any.iter().map(|clause| evaluate(clause, &mut hits)).collect::<Vec<_>>().into_iter())
    };
    let all = and(rule.all.iter().map(|clause| evaluate(clause, &mut hits)).collect::<Vec<_>>().into_iter());
    let none =
        or(rule.none.iter().map(|clause| evaluate(clause, &mut exclusions)).collect::<Vec<_>>().into_iter());
    let outcome = match and([any, all, none.map(|excluded| !excluded)].into_iter()) {
        Some(true) => Outcome::Match,
        Some(false) => Outcome::NoMatch,
        None => Outcome::NotEvaluated,
    };
    if outcome != Outcome::Match {
        hits.clear();
    }
    RuleResult {
        rule: rule.id.clone(),
        outcome,
        review_priority: rule.review_priority,
        hits,
        exclusions,
        not_evaluated: missing,
    }
}

/// Une clause : vraie dès qu'un terme apparaît dans un champ connu ; inconnue si
/// aucun champ connu ne la satisfait et qu'un champ manque ; fausse sinon.
fn evaluate_clause(
    clause: &Clause,
    normalization: &Normalization,
    input: &Map<String, Value>,
) -> (Truth, Vec<Hit>, Vec<String>) {
    let terms: Vec<(String, &str)> = clause
        .terms
        .iter()
        .map(|term| (normalize(term, normalization).text.trim().to_owned(), term.as_str()))
        .collect();
    let mut hits = Vec::new();
    let mut missing = Vec::new();
    for field in &clause.fields {
        let values: Vec<(Option<usize>, &str)> = match input.get(field) {
            None => {
                missing.push(field.clone());
                continue;
            }
            Some(Value::String(text)) => vec![(None, text.as_str())],
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| Some((Some(index), item.as_str()?)))
                .collect(),
            Some(_) => Vec::new(),
        };
        for (index, original) in values {
            let text = normalize(original, normalization);
            for (term, written) in &terms {
                for start in occurrences(&text.text, term, clause.mode) {
                    if hits.len() >= MAX_HITS {
                        break;
                    }
                    let (from, to) = text.original_span(start, start + term.len());
                    hits.push(Hit {
                        field: field.clone(),
                        index,
                        term: (*written).to_owned(),
                        start: from,
                        end: to,
                        excerpt: excerpt(original, from, to),
                    });
                }
            }
        }
    }
    let truth = if !hits.is_empty() {
        Some(true)
    } else if missing.is_empty() {
        Some(false)
    } else {
        None
    };
    (truth, hits, missing)
}

/// Texte normalisé ; `origin[i]` est le caractère d'origine de l'octet normalisé `i`.
#[derive(Debug)]
struct Normalized {
    text: String,
    origin: Vec<(usize, usize)>,
}

impl Normalized {
    /// Plage d'origine (octets) des octets normalisés `start..end`.
    fn original_span(&self, start: usize, end: usize) -> (usize, usize) {
        let from = self.origin.get(start).map_or(0, |span| span.0);
        let to = end.checked_sub(1).and_then(|last| self.origin.get(last)).map_or(from, |span| span.1);
        (from, to)
    }
}

fn normalize(input: &str, settings: &Normalization) -> Normalized {
    let mut text = String::with_capacity(input.len());
    let mut origin = Vec::with_capacity(input.len());
    for (offset, character) in input.char_indices() {
        let span = (offset, offset + character.len_utf8());
        let push = |output: char, text: &mut String, origin: &mut Vec<(usize, usize)>| {
            if output.is_whitespace() {
                if text.ends_with(' ') || text.is_empty() {
                    return;
                }
                text.push(' ');
                origin.push(span);
                return;
            }
            text.push(output);
            origin.extend(std::iter::repeat_n(span, output.len_utf8()));
        };
        decompose_compatible(character, |decomposed| {
            if settings.strip_accents && is_combining_mark(decomposed) {
                return;
            }
            if settings.case_sensitive {
                push(decomposed, &mut text, &mut origin);
            } else {
                for lower in decomposed.to_lowercase() {
                    push(lower, &mut text, &mut origin);
                }
            }
        });
    }
    Normalized { text, origin }
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || is_combining_mark(character)
}

/// Positions (octets normalisés) des occurrences valides d'un terme.
fn occurrences(text: &str, term: &str, mode: MatchMode) -> Vec<usize> {
    if term.is_empty() {
        return Vec::new();
    }
    text.match_indices(term)
        .map(|(start, _)| start)
        .filter(|&start| {
            mode == MatchMode::Substring || {
                let before = text[..start].chars().next_back().is_none_or(|c| !is_word(c));
                let after = text[start + term.len()..].chars().next().is_none_or(|c| !is_word(c));
                before && after
            }
        })
        .collect()
}

/// Extrait du texte d'origine autour d'une occurrence.
fn excerpt(original: &str, start: usize, end: usize) -> String {
    let from = original[..start].char_indices().rev().nth(EXCERPT_CHARS - 1).map_or(0, |(index, _)| index);
    let to =
        original[end..].char_indices().nth(EXCERPT_CHARS).map_or(original.len(), |(index, _)| end + index);
    let mut text = String::new();
    if from > 0 {
        text.push('…');
    }
    text.push_str(&original[from..to]);
    if to < original.len() {
        text.push('…');
    }
    text
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
