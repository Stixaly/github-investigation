//! Identifiants, mots de passe et motifs de recherche saisis par les analystes.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("Login : 3 à 40 caractères alphanumériques, point, tiret ou underscore")]
    Login,
    #[error("Le mot de passe doit contenir entre 14 et 1024 caractères")]
    Password,
}

/// Login d'un compte local, normalisé en minuscules.
pub fn normalize_login(login: &str) -> Result<String, CredentialError> {
    let valid = (3..=40).contains(&login.len())
        && login.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if valid { Ok(login.to_ascii_lowercase()) } else { Err(CredentialError::Login) }
}

/// Longueur exigée pour un nouveau mot de passe, comptée en caractères.
pub fn check_new_password(password: &str) -> Result<(), CredentialError> {
    if (14..=1024).contains(&password.chars().count()) { Ok(()) } else { Err(CredentialError::Password) }
}

/// Échappe `\`, `%` et `_` pour un motif `LIKE ... ESCAPE '\'`.
///
/// Sans cela, un filtre `_` ou `%` saisi par l'analyste correspondrait à
/// n'importe quel libellé.
pub fn escape_like(pattern: &str) -> String {
    let mut escaped = String::with_capacity(pattern.len());
    for character in pattern.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Vérifie qu'un champ texte compte entre `min` et `max` caractères.
pub fn within(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logins_are_normalized_or_refused() {
        assert_eq!(normalize_login("Alice.B-2_"), Ok("alice.b-2_".into()));
        for login in ["ab", "a b c", "élodie", "x".repeat(41).as_str(), "semi;colon"] {
            assert_eq!(normalize_login(login), Err(CredentialError::Login), "{login}");
        }
    }

    #[test]
    fn passwords_need_fourteen_characters() {
        assert!(check_new_password(&"a".repeat(13)).is_err());
        assert!(check_new_password(&"a".repeat(14)).is_ok());
        assert!(check_new_password(&"é".repeat(14)).is_ok());
        assert!(check_new_password(&"x".repeat(1025)).is_err());
    }

    #[test]
    fn like_wildcards_are_literal() {
        assert_eq!(escape_like(r"50%_off\now"), r"50\%\_off\\now");
    }
}
