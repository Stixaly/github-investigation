//! Découverte GitHub par recherche de dépôts ou de code : un plan de requêtes
//! compilé et affiché avant tout appel, et la subdivision des recherches saturées.
//!
//! Une recherche distante ne doit jamais éliminer silencieusement ce qu'une
//! règle locale aurait retenu : un terme donne sa propre requête, les
//! conjonctions et exclusions sont vérifiées localement, et ce que GitHub ne
//! sait pas chercher (sous-chaînes, champs de compte) est signalé dans le plan.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::rules::{Clause, Definition, MatchMode};

/// Résultats accessibles au plus par requête de recherche GitHub.
pub const MAX_RESULTS: u64 = 1000;
/// Éléments par page de recherche.
pub const PER_PAGE: u64 = 100;
/// Longueur maximale d'une requête acceptée par GitHub.
pub const MAX_QUERY_CHARS: usize = 256;
/// Opérateurs `AND`, `OR`, `NOT` au plus dans une requête GitHub.
pub const MAX_OPERATORS: usize = 5;
/// Requêtes au plus dans un plan, subdivisions non comprises.
pub const MAX_QUERIES: usize = 20;
pub const MAX_PAGES: u32 = 10;
pub const MAX_SUBDIVISIONS: u32 = 8;
/// Fichiers indexés par la recherche de code : moins de 384 Ko.
pub const CODE_MAX_BYTES: u64 = 384 * 1024;
/// Création du premier dépôt possible, en jours depuis 1970-01-01 (2008-01-01).
pub const FIRST_DAY: i64 = 13_879;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Repositories,
    Code,
}

impl SearchKind {
    /// Ressource de quota GitHub de cette recherche.
    pub fn resource(self) -> &'static str {
        match self {
            Self::Repositories => "search",
            Self::Code => "code_search",
        }
    }

    pub fn path(self) -> &'static str {
        match self {
            Self::Repositories => "/search/repositories",
            Self::Code => "/search/code",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Repositories => "Dépôts",
            Self::Code => "Code",
        }
    }
}

/// Partie d'une requête saturée : période de création (dépôts) ou taille de fichier (code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
pub enum Slice {
    /// Jours depuis 1970-01-01, bornes comprises.
    Created { from: i64, to: i64 },
    /// Octets, bornes comprises.
    Size { from: u64, to: u64 },
}

impl Slice {
    fn qualifier(&self) -> String {
        match self {
            Self::Created { from, to } => format!("created:{}..{}", date(*from), date(*to)),
            Self::Size { from, to } => format!("size:{from}..{to}"),
        }
    }

    fn halves(&self) -> Option<[Self; 2]> {
        match *self {
            Self::Created { from, to } if to > from => {
                let middle = from + (to - from) / 2;
                Some([Self::Created { from, to: middle }, Self::Created { from: middle + 1, to }])
            }
            Self::Size { from, to } if to > from => {
                let middle = from + (to - from) / 2;
                Some([Self::Size { from, to: middle }, Self::Size { from: middle + 1, to }])
            }
            _ => None,
        }
    }
}

/// Requête du plan, telle qu'envoyée à GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    /// Texte envoyé, qualificatifs compris.
    pub q: String,
    /// Requête sans qualificatif de partition, base des subdivisions.
    pub base: String,
    /// Requête saisie, ou règle et terme d'une liste.
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice: Option<Slice>,
}

impl Query {
    fn new(base: String, origin: String) -> Self {
        Self { q: base.clone(), base, origin, slice: None }
    }

    /// Deux sous-requêtes couvrant la même recherche, ou `None` si elle ne se divise plus.
    ///
    /// `today` : jour courant (jours depuis 1970-01-01), borne des périodes de création.
    pub fn subdivide(&self, kind: SearchKind, today: i64) -> Option<[Self; 2]> {
        let whole = match kind {
            SearchKind::Repositories if !mentions(&self.base, "created:") => {
                Slice::Created { from: FIRST_DAY, to: today.max(FIRST_DAY) }
            }
            SearchKind::Code if !mentions(&self.base, "size:") => Slice::Size { from: 0, to: CODE_MAX_BYTES },
            _ => return None,
        };
        let halves = self.slice.as_ref().unwrap_or(&whole).halves()?;
        Some(halves.map(|slice| Self {
            q: format!("{} {}", self.base, slice.qualifier()),
            base: self.base.clone(),
            origin: self.origin.clone(),
            slice: Some(slice),
        }))
    }
}

fn mentions(query: &str, qualifier: &str) -> bool {
    query.split_whitespace().any(|word| word.trim_start_matches('-').starts_with(qualifier))
}

/// Liste d'origine d'un plan compilé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRef {
    pub id: String,
    pub version: i32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub kind: SearchKind,
    pub include_forks: bool,
    pub max_pages: u32,
    pub max_subdivisions: u32,
}

impl Options {
    fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_PAGES).contains(&self.max_pages) {
            return Err(format!("Pages par requête : 1 à {MAX_PAGES}"));
        }
        if self.max_subdivisions > MAX_SUBDIVISIONS {
            return Err(format!("Subdivisions : {MAX_SUBDIVISIONS} au plus"));
        }
        Ok(())
    }
}

/// Plan d'une découverte : requêtes distantes, limites et ce qui reste à vérifier localement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub kind: SearchKind,
    pub queries: Vec<Query>,
    pub max_pages: u32,
    pub max_subdivisions: u32,
    pub include_forks: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<ListRef>,
    /// Ce que la recherche distante ne vérifie pas et que le moteur local vérifiera.
    pub local_checks: Vec<String>,
    /// Limites de GitHub et écarts entre la recherche distante et les règles.
    pub warnings: Vec<String>,
}

impl Plan {
    /// Majorant des requêtes : chaque subdivision remplace une requête par deux.
    pub fn max_requests(&self) -> u64 {
        let queries = self.queries.len() as u64 + 2 * u64::from(self.max_subdivisions);
        queries * u64::from(self.max_pages)
    }

    /// Libellé court du lancement, affiché comme seed du scan.
    pub fn label(&self) -> String {
        let subject = match (&self.list, self.queries.first()) {
            (Some(list), _) => format!("liste « {} » v{}", list.name, list.version),
            (None, Some(query)) => query.base.clone(),
            (None, None) => String::new(),
        };
        let label = format!("{} : {subject}", self.kind.label());
        label.chars().take(250).collect()
    }
}

fn notices(kind: SearchKind) -> Vec<String> {
    let mut notices = vec![format!(
        "GitHub ne donne accès qu'aux {MAX_RESULTS} premiers résultats d'une recherche : au-delà, la recherche est subdivisée dans la limite fixée, sinon marquée partielle."
    )];
    if kind == SearchKind::Code {
        notices.push(
            "Recherche de code : token GitHub requis ; branche par défaut et fichiers de moins de 384 Ko seulement ; \
             un fork n'est indexé que s'il a plus d'étoiles que son dépôt d'origine."
                .to_owned(),
        );
        notices.push(
            "Les extraits de code ne sont pas conservés tant que l'acquisition de contenus n'est pas activée (cadrage RGPD, REM-30) : \
             seuls le dépôt, le chemin et l'empreinte du fichier sont enregistrés."
                .to_owned(),
        );
    }
    notices
}

fn check_query(query: &str) -> Result<(), String> {
    if query.trim().is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("Requête : 1 à {MAX_QUERY_CHARS} caractères"));
    }
    if query.chars().any(char::is_control) {
        return Err("Requête : caractères de contrôle refusés".into());
    }
    let operators = query.split_whitespace().filter(|word| matches!(*word, "AND" | "OR" | "NOT")).count();
    if operators > MAX_OPERATORS {
        return Err(format!("Requête : {MAX_OPERATORS} opérateurs AND, OR ou NOT au plus (limite GitHub)"));
    }
    Ok(())
}

fn with_forks(query: String, options: Options) -> String {
    if options.include_forks && !mentions(&query, "fork:") { format!("{query} fork:true") } else { query }
}

/// Plan d'une requête saisie telle quelle (syntaxe de recherche GitHub).
pub fn from_query(text: &str, options: Options) -> Result<Plan, String> {
    options.validate()?;
    let typed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    check_query(&typed)?;
    let base = with_forks(typed, options);
    check_query(&base)?;
    Ok(Plan {
        kind: options.kind,
        queries: vec![Query::new(base, "requête saisie".into())],
        max_pages: options.max_pages,
        max_subdivisions: options.max_subdivisions,
        include_forks: options.include_forks,
        list: None,
        local_checks: Vec::new(),
        warnings: notices(options.kind),
    })
}

/// Qualificatif `in:` d'un champ de liste, s'il est cherchable dans les dépôts.
fn searchable(field: &str) -> Option<&'static str> {
    match field {
        "repository.name" | "repository.full_name" => Some("name"),
        "repository.description" => Some("description"),
        "repository.topics" => Some("topics"),
        "repository.readme" => Some("readme"),
        _ => None,
    }
}

/// Plan d'une liste publiée : une requête de dépôts par terme des clauses positives.
pub fn from_list(definition: &Definition, list: ListRef, options: Options) -> Result<Plan, String> {
    options.validate()?;
    if options.kind != SearchKind::Repositories {
        return Err(
            "Une liste se recherche dans les dépôts ; la recherche de code part d'une requête saisie".into(),
        );
    }
    let mut queries: Vec<Query> = Vec::new();
    let mut local_checks = Vec::new();
    let mut warnings = notices(options.kind);
    for rule in definition.rules.iter().filter(|rule| rule.enabled) {
        for clause in rule.any.iter().chain(&rule.all) {
            plan_clause(&rule.id, clause, options, &mut queries, &mut warnings);
        }
        if !rule.all.is_empty() {
            local_checks.push(format!(
                "Règle « {} » : toutes les clauses « all » sont vérifiées localement.",
                rule.id
            ));
        }
        if !rule.none.is_empty() {
            local_checks
                .push(format!("Règle « {} » : les exclusions « none » sont vérifiées localement.", rule.id));
        }
    }
    if queries.is_empty() {
        return Err("Aucun terme de la liste n'est cherchable dans les dépôts GitHub".into());
    }
    if queries.len() > MAX_QUERIES {
        return Err(format!(
            "{} requêtes : {MAX_QUERIES} au plus par découverte, réduire la liste",
            queries.len()
        ));
    }
    Ok(Plan {
        kind: options.kind,
        queries,
        max_pages: options.max_pages,
        max_subdivisions: options.max_subdivisions,
        include_forks: options.include_forks,
        list: Some(list),
        local_checks,
        warnings,
    })
}

fn plan_clause(
    rule: &str,
    clause: &Clause,
    options: Options,
    queries: &mut Vec<Query>,
    warnings: &mut Vec<String>,
) {
    let mut fields: Vec<&str> = Vec::new();
    for field in clause.fields.iter().filter_map(|field| searchable(field)) {
        if !fields.contains(&field) {
            fields.push(field);
        }
    }
    let skipped: Vec<&str> =
        clause.fields.iter().map(String::as_str).filter(|field| searchable(field).is_none()).collect();
    if !skipped.is_empty() {
        warnings.push(format!(
            "Règle « {rule} » : {} non cherchables sur GitHub, vérifiés localement.",
            skipped.join(", ")
        ));
    }
    if fields.is_empty() {
        return;
    }
    if clause.mode == MatchMode::Substring {
        warnings.push(format!(
            "Règle « {rule} » : GitHub ne cherche que des mots ; les sous-chaînes ({}) ne seront trouvées que si elles forment un mot, la recherche distante est plus restrictive que la règle.",
            clause.terms.join(", ")
        ));
    }
    for term in &clause.terms {
        if term.contains('"') {
            warnings.push(format!(
                "Règle « {rule} » : « {term} » contient un guillemet et n'est pas cherché sur GitHub."
            ));
            continue;
        }
        let base = with_forks(format!("\"{}\" in:{}", term.trim(), fields.join(",")), options);
        if check_query(&base).is_err() {
            warnings.push(format!("Règle « {rule} » : « {term} » dépasse la longueur d'une requête GitHub."));
            continue;
        }
        if !queries.iter().any(|query| query.base == base) {
            queries.push(Query::new(base, format!("{rule} : {term}")));
        }
    }
}

/// URL d'une page de recherche, paramètres encodés.
pub fn search_url(api: &str, kind: SearchKind, query: &str) -> String {
    format!("{api}{}?q={}&per_page={PER_PAGE}", kind.path(), encode(query))
}

/// Encodage d'un paramètre d'URL (RFC 3986, caractères non réservés conservés).
pub fn encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// Date civile `AAAA-MM-JJ` d'un nombre de jours depuis 1970-01-01 (algorithme de H. Hinnant).
pub fn date(days: i64) -> String {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
