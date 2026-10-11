//! Point de départ d'un scan : un compte ou un dépôt GitHub public.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SeedKind {
    Account,
    Repository,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    pub kind: SeedKind,
    /// `compte` ou `compte/depot`, casse d'origine conservée.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SeedError {
    #[error("Utiliser une URL https://github.com/compte[/depot] sans paramètres")]
    Url,
    #[error("Seed attendue : compte ou compte/depot")]
    Shape,
    #[error("Nom de dépôt incorrect")]
    Repository,
}

/// Accepte `compte`, `compte/depot`, `github.com/compte/depot` ou
/// `https://github.com/compte/depot`. Toute autre origine, un port, des
/// identifiants, une requête, un fragment ou un chemin plus profond sont refusés.
pub fn parse_seed(input: &str) -> Result<Seed, SeedError> {
    let value = input.trim().trim_end_matches('/');
    let path = if let Some((scheme, rest)) = value.split_once("://") {
        if !scheme.eq_ignore_ascii_case("https") {
            return Err(SeedError::Url);
        }
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if !authority.eq_ignore_ascii_case("github.com") || tail.contains(['?', '#']) {
            return Err(SeedError::Url);
        }
        tail.trim_matches('/')
    } else if value.get(..11).is_some_and(|prefix| prefix.eq_ignore_ascii_case("github.com/")) {
        // A login cannot contain a dot, so this prefix is never an account name.
        &value[11..]
    } else {
        value
    };
    let parts: Vec<&str> = path.split('/').collect();
    if !(1..=2).contains(&parts.len()) || !is_login(parts[0]) {
        return Err(SeedError::Shape);
    }
    if let Some(repository) = parts.get(1)
        && !is_repository(repository)
    {
        return Err(SeedError::Repository);
    }
    let kind = if parts.len() == 2 { SeedKind::Repository } else { SeedKind::Account };
    Ok(Seed { kind, path: parts.join("/") })
}

/// GitHub login: 1 to 39 ASCII letters, digits or hyphens, starting with a letter or digit.
fn is_login(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && value.len() <= 39
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn is_repository(value: &str) -> bool {
    (1..=100).contains(&value.len())
        && value != "."
        && value != ".."
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_accounts_repositories_and_urls() {
        let cases = [
            ("octocat", SeedKind::Account, "octocat"),
            ("https://github.com/octocat/Hello-World/", SeedKind::Repository, "octocat/Hello-World"),
            ("owner/repo", SeedKind::Repository, "owner/repo"),
            ("  HTTPS://GitHub.com/owner/repo.js  ", SeedKind::Repository, "owner/repo.js"),
            ("github.com/compte/depot", SeedKind::Repository, "compte/depot"),
            ("GitHub.com/compte", SeedKind::Account, "compte"),
        ];
        for (input, kind, path) in cases {
            assert_eq!(parse_seed(input), Ok(Seed { kind, path: path.into() }), "{input}");
        }
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_seeds() {
        for input in [
            "https://evil.example/u/r",
            "https://github.com@evil.example/u/r",
            "https://github.com:8443/u/r",
            "https://github.com/u/r?q=secret",
            "https://github.com/u/r#readme",
            "owner/../x",
            "owner/..",
            "/etc/passwd",
            "http://github.com/u/r",
            "https://github.com/u/r/tree/main",
            "github.com",
            "",
            "-leading-hyphen",
            "a234567890123456789012345678901234567890",
            "owner/repo name",
            "owner//repo",
            "ownér/repo",
        ] {
            assert!(parse_seed(input).is_err(), "{input:?} should be rejected");
        }
    }

    #[test]
    fn reports_the_failing_part() {
        assert_eq!(parse_seed("http://github.com/u"), Err(SeedError::Url));
        assert_eq!(parse_seed("a/b/c"), Err(SeedError::Shape));
        assert_eq!(parse_seed("owner/.."), Err(SeedError::Repository));
    }
}
