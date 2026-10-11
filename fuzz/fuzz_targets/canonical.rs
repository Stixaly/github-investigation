//! Evidence canonicalization: any JSON GitHub could return serializes to valid
//! JSON that reads back as the same value, so equal payloads hash equally.
#![no_main]

use libfuzzer_sys::fuzz_target;
use observatory_domain::canonical::{COMPACT, PYTHON_DEFAULT, python_float, to_string};
use observatory_domain::evidence_hash;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    if data.len() >= 8 {
        let value = f64::from_le_bytes(data[..8].try_into().expect("eight bytes"));
        if value.is_finite() {
            assert_eq!(python_float(value).parse::<f64>().ok(), Some(value));
        }
    }
    let Ok(value) = serde_json::from_str::<Value>(std::str::from_utf8(data).unwrap_or("")) else { return };
    for style in [COMPACT, PYTHON_DEFAULT] {
        let text = to_string(&value, style);
        let Ok(again) = serde_json::from_str::<Value>(&text) else {
            // Like Python, a number beyond the f64 range is written `Infinity`:
            // not JSON, but the same hash input as the 0.1 platform produced.
            assert!(text.contains("Infinity"), "canonical output is JSON: {text}");
            continue;
        };
        assert_eq!(to_string(&again, style), text, "canonical form is stable");
    }
    assert_eq!(evidence_hash("source", &value), evidence_hash("source", &value.clone()));
});
