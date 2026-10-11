//! Suivi de la pagination GitHub sans quitter l'API attendue.

/// Seule origine que le collecteur interroge en production.
pub const GITHUB_API: &str = "https://api.github.com";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Lien de pagination non autorisé")]
pub struct ForbiddenLink;

/// Chemin d'une URL située sous `base`, sans la requête.
///
/// `None` si l'URL sort de `base` : autre hôte, port, identifiants, fragment,
/// espace ou caractère de contrôle.
pub fn api_path<'a>(url: &'a str, base: &str) -> Option<&'a str> {
    let rest = url.strip_prefix(base)?;
    if !rest.starts_with('/')
        || rest.contains('#')
        || rest.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    rest.split('?').next()
}

/// URL de la page suivante annoncée par l'en-tête `Link`.
///
/// La page suivante doit rester sur le même chemin que la page courante, ou sur
/// le chemin canonique `/repositories/<id>/...` que GitHub utilise pour paginer
/// un dépôt. Tout autre lien est refusé plutôt qu'ignoré.
pub fn next_page(
    link_header: &str,
    current_url: &str,
    canonical_path: Option<&str>,
    base: &str,
) -> Result<Option<String>, ForbiddenLink> {
    for part in link_header.split(',') {
        let mut segments = part.split(';');
        let target = segments.next().unwrap_or_default().trim();
        if !segments.any(|segment| segment.trim() == r#"rel="next""#) {
            continue;
        }
        let url = target.trim_start_matches('<').trim_end_matches('>');
        let path = api_path(url, base).ok_or(ForbiddenLink)?;
        if api_path(current_url, base) == Some(path) || canonical_path == Some(path) {
            return Ok(Some(url.to_owned()));
        }
        return Err(ForbiddenLink);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORKS: &str = "https://api.github.com/repos/a/b/forks";

    #[test]
    fn follows_the_same_path() {
        let link = r#"<https://api.github.com/repos/a/b/forks?page=2>; rel="next", <https://api.github.com/repos/a/b/forks?page=9>; rel="last""#;
        let next = next_page(link, &format!("{FORKS}?page=1"), None, GITHUB_API).unwrap();
        assert_eq!(next.as_deref(), Some("https://api.github.com/repos/a/b/forks?page=2"));
    }

    #[test]
    fn follows_the_canonical_repository_path() {
        let link = r#"<https://api.github.com/repositories/42/forks?page=2>; rel="next""#;
        let next = next_page(link, FORKS, Some("/repositories/42/forks"), GITHUB_API).unwrap();
        assert!(next.unwrap().ends_with("page=2"));
        assert_eq!(next_page(link, FORKS, Some("/repositories/99/forks"), GITHUB_API), Err(ForbiddenLink));
    }

    #[test]
    fn refuses_other_origins() {
        for link in [
            r#"<https://evil.test/steal>; rel="next""#,
            r#"<https://api.github.com.evil.test/repos/a/b/forks>; rel="next""#,
            r#"<https://api.github.com@evil.test/repos/a/b/forks>; rel="next""#,
            r#"<https://api.github.com:444/repos/a/b/forks>; rel="next""#,
            r#"<http://api.github.com/repos/a/b/forks>; rel="next""#,
            r#"<https://api.github.com/repos/a/b/forks#x>; rel="next""#,
            r#"<https://api.github.com/user/repos>; rel="next""#,
        ] {
            assert_eq!(next_page(link, FORKS, None, GITHUB_API), Err(ForbiddenLink), "{link}");
        }
    }

    #[test]
    fn last_page_has_no_next() {
        assert_eq!(next_page("", FORKS, None, GITHUB_API), Ok(None));
        let link = r#"<https://api.github.com/repos/a/b/forks?page=1>; rel="prev""#;
        assert_eq!(next_page(link, FORKS, None, GITHUB_API), Ok(None));
    }
}
