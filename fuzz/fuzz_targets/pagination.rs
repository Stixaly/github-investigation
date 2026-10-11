//! A `Link` header returned by the API: a followed link always stays on the
//! expected origin and on the current (or canonical) path.
#![no_main]

use libfuzzer_sys::fuzz_target;
use observatory_domain::pagination::{api_path, next_page};

const BASE: &str = "https://api.github.com";
const CURRENT: &str = "https://api.github.com/repos/a/b/forks?per_page=50";
const CANONICAL: &str = "/repositories/42/forks";

fuzz_target!(|header: &str| {
    if let Ok(Some(url)) = next_page(header, CURRENT, Some(CANONICAL), BASE) {
        let path = api_path(&url, BASE).expect("a followed link is below the API base");
        assert!(path == "/repos/a/b/forks" || path == CANONICAL, "{url}");
        assert!(url.starts_with("https://api.github.com/"));
        assert!(!url.contains('#') && !url.chars().any(char::is_whitespace));
    }
});
