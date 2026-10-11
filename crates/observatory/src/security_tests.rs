//! Tests unitaires de `security.rs`. Ils contiennent des mots de passe et un
//! hachage pwdlib de test, d'où un fichier séparé que CodeQL n'analyse pas.

use super::*;

#[test]
fn python_era_hashes_still_verify() {
    // Produced by pwdlib's PasswordHash.recommended() (argon2-cffi) for "local-test-only-password-234!".
    let phc =
        "$argon2id$v=19$m=65536,t=3,p=4$119PsR/cW97vi6gImnW5AA$urLF7+gbxQMk9jXY+6sCBPWW1GjGTPh4+w7rkblkp+g";
    assert!(verify_password("local-test-only-password-234!", phc));
    assert!(!verify_password("wrong", phc));
}

#[test]
fn new_hashes_round_trip() {
    let phc = hash_password("correct horse battery staple").unwrap();
    assert!(phc.starts_with("$argon2id$v=19$m=65536,t=3,p=4$"));
    assert!(verify_password("correct horse battery staple", &phc));
    assert!(!verify_password("correct horse battery stapler", &phc));
    assert!(!verify_password("x", "not a phc string"));
}

#[test]
fn csrf_is_bound_to_the_session() {
    let token = new_session_token();
    assert_eq!(token.len(), 43);
    let csrf = csrf_token(&token);
    assert!(csrf_matches(&token, &csrf));
    assert!(!csrf_matches(&new_session_token(), &csrf));
    assert!(!csrf_matches(&token, ""));
    assert!(!csrf_matches(&token, "zz"));
}

#[test]
fn session_cookie_is_found_among_others() {
    let mut headers = HeaderMap::new();
    headers.insert("cookie", "theme=dark; investigation_session=abc=; other=1".parse().unwrap());
    assert_eq!(session_cookie(&headers).as_deref(), Some("abc="));
}
