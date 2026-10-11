//! A seed typed by an analyst: never panics, and an accepted seed is a plain
//! `owner[/repository]` path that parses back to itself.
#![no_main]

use libfuzzer_sys::fuzz_target;
use observatory_domain::parse_seed;

fuzz_target!(|input: &str| {
    if let Ok(seed) = parse_seed(input) {
        assert!(seed.path.split('/').count() <= 2);
        assert!(seed.path.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')));
        assert_eq!(parse_seed(&seed.path), Ok(seed.clone()));
        assert_eq!(parse_seed(&format!("https://github.com/{}", seed.path)), Ok(seed));
    }
});
