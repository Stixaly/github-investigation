//! Pivot compte → dépôts par l'API GraphQL (accès authentifié seulement).
//!
//! Une page de 100 dépôts coûte un point GraphQL, comme une page REST coûte une
//! requête, mais elle apporte aussi le dépôt d'origine de chaque fork, que la
//! liste REST n'indique pas. Les nœuds sont gardés tels que reçus comme preuves ;
//! leur forme REST sert seulement à créer les entités et relations habituelles.

use serde_json::{Map, Value, json};

/// Ressource de quota de l'API GraphQL.
pub const GRAPHQL: &str = "graphql";

/// Dépôts publics possédés par un compte ou une organisation, du plus récemment poussé au plus ancien.
pub const REPOSITORIES: &str = "query($login: String!, $after: String) { \
repositoryOwner(login: $login) { \
repositories(first: 100, after: $after, ownerAffiliations: [OWNER], privacy: PUBLIC, \
orderBy: {field: PUSHED_AT, direction: DESC}) { \
pageInfo { hasNextPage endCursor } nodes { ...Repo parent { ...Repo } } } } } \
fragment Repo on Repository { databaseId nameWithOwner isPrivate isFork isArchived description url \
createdAt pushedAt stargazerCount forkCount \
owner { __typename login ... on User { databaseId } ... on Organization { databaseId } } }";

/// Page de dépôts lue dans une réponse GraphQL.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub nodes: Vec<Value>,
    /// Curseur de la page suivante, s'il y en a une.
    pub next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PageError {
    #[error("Quota GraphQL atteint")]
    RateLimited,
    #[error("Erreur GraphQL : {0}")]
    Failed(String),
}

/// Lit une réponse GraphQL : erreurs, compte introuvable, nœuds et curseur.
pub fn page(payload: &Value) -> Result<Page, PageError> {
    if let Some(errors) = payload.get("errors").and_then(Value::as_array).filter(|errors| !errors.is_empty())
    {
        let kinds: Vec<&str> = errors
            .iter()
            .map(|error| error.get("type").and_then(Value::as_str).unwrap_or("inconnue"))
            .collect();
        if kinds.contains(&"RATE_LIMITED") {
            return Err(PageError::RateLimited);
        }
        return Err(PageError::Failed(kinds.join(", ")));
    }
    let owner = payload.pointer("/data/repositoryOwner").filter(|owner| !owner.is_null());
    let Some(repositories) = owner.and_then(|owner| owner.get("repositories")) else {
        return Err(PageError::Failed("compte introuvable".into()));
    };
    let Some(nodes) = repositories.get("nodes").and_then(Value::as_array) else {
        return Err(PageError::Failed("liste de dépôts attendue".into()));
    };
    let info = repositories.get("pageInfo");
    let has_next = info.and_then(|info| info.get("hasNextPage")).and_then(Value::as_bool) == Some(true);
    let cursor = info.and_then(|info| info.get("endCursor")).and_then(Value::as_str);
    Ok(Page {
        nodes: nodes.iter().filter(|node| node.is_object()).cloned().collect(),
        next: cursor.filter(|_| has_next).map(str::to_owned),
    })
}

/// Forme REST d'un nœud `Repository` : mêmes clés que `GET /repos/{owner}/{name}`.
pub fn rest_repository(node: &Value) -> Option<Value> {
    let node = node.as_object()?;
    let mut rest = Map::new();
    for (graphql, key) in [
        ("databaseId", "id"),
        ("nameWithOwner", "full_name"),
        ("isPrivate", "private"),
        ("isFork", "fork"),
        ("isArchived", "archived"),
        ("description", "description"),
        ("url", "html_url"),
        ("createdAt", "created_at"),
        ("pushedAt", "pushed_at"),
        ("stargazerCount", "stargazers_count"),
        ("forkCount", "forks_count"),
    ] {
        if let Some(value) = node.get(graphql) {
            rest.insert(key.into(), value.clone());
        }
    }
    if let Some(owner) = node.get("owner").and_then(Value::as_object) {
        rest.insert(
            "owner".into(),
            json!({"id": owner.get("databaseId"), "login": owner.get("login"), "type": owner.get("__typename")}),
        );
    }
    if let Some(parent) = node.get("parent").and_then(rest_repository) {
        rest.insert("parent".into(), parent);
    }
    Some(Value::Object(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, name: &str, parent: &Value) -> Value {
        json!({"databaseId": id, "nameWithOwner": name, "isPrivate": false, "isFork": !parent.is_null(),
               "description": null, "stargazerCount": 3, "parent": parent,
               "owner": {"__typename": "User", "login": name.split('/').next(), "databaseId": 2}})
    }

    #[test]
    fn nodes_take_the_rest_shape() {
        let upstream = node(7, "bob/upstream", &Value::Null);
        let fork = rest_repository(&node(5, "alice/upstream", &upstream)).unwrap();
        assert_eq!(fork["id"], 5);
        assert_eq!(fork["full_name"], "alice/upstream");
        assert_eq!(fork["fork"], true);
        assert_eq!(fork["stargazers_count"], 3);
        assert_eq!(fork["owner"], json!({"id": 2, "login": "alice", "type": "User"}));
        assert_eq!(fork["parent"]["full_name"], "bob/upstream");
        assert!(fork["parent"].get("parent").is_none());
        assert_eq!(rest_repository(&json!("not an object")), None);
    }

    #[test]
    fn pages_follow_the_cursor() {
        let payload = json!({"data": {"repositoryOwner": {"repositories": {
            "pageInfo": {"hasNextPage": true, "endCursor": "c1"},
            "nodes": [node(5, "alice/a", &Value::Null), null]}}}});
        assert_eq!(
            page(&payload),
            Ok(Page { nodes: vec![node(5, "alice/a", &Value::Null)], next: Some("c1".into()) })
        );
        let last = json!({"data": {"repositoryOwner": {"repositories": {
            "pageInfo": {"hasNextPage": false, "endCursor": "c2"}, "nodes": []}}}});
        assert_eq!(page(&last).unwrap().next, None);
    }

    #[test]
    fn errors_are_explained() {
        assert_eq!(
            page(&json!({"data": {"repositoryOwner": null}})),
            Err(PageError::Failed("compte introuvable".into()))
        );
        assert_eq!(page(&json!({"errors": [{"type": "RATE_LIMITED"}]})), Err(PageError::RateLimited));
        assert_eq!(
            page(&json!({"errors": [{"type": "NOT_FOUND"}, {"message": "x"}]})).unwrap_err().to_string(),
            "Erreur GraphQL : NOT_FOUND, inconnue"
        );
        assert!(page(&json!({"data": {"repositoryOwner": {"repositories": {}}}})).is_err());
    }
}
