//! Forme canonique des preuves JSON et empreinte `evidence_hash`.
//!
//! Le format est celui que produisait le socle Python 0.1 :
//! `json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.
//! Le conserver à l'octet près garde la déduplication des preuves déjà
//! enregistrées : une même réponse GitHub, observée avant et après la migration,
//! aboutit à la même ligne `evidence`. Voir docs/adr/0002-backend-rust.md.

use std::fmt::Write as _;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Séparateurs et échappement d'une sérialisation à clés triées.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    item_separator: &'static str,
    key_separator: &'static str,
    ensure_ascii: bool,
}

/// `json.dumps(..., sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.
pub const COMPACT: Style = Style { item_separator: ",", key_separator: ":", ensure_ascii: false };

/// `json.dumps(..., sort_keys=True)` : séparateurs et échappement ASCII par défaut de Python.
pub const PYTHON_DEFAULT: Style = Style { item_separator: ", ", key_separator: ": ", ensure_ascii: true };

/// Empreinte d'une preuve : SHA-256 hexadécimal de `source + "\n" + JSON canonique`.
pub fn evidence_hash(source: &str, payload: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hasher.update(b"\n");
    hasher.update(to_string(payload, COMPACT).as_bytes());
    hex::encode(hasher.finalize())
}

/// SHA-256 hexadécimal d'un texte.
pub fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Sérialise `value` avec des clés d'objet triées par point de code.
pub fn to_string(value: &Value, style: Style) -> String {
    let mut out = String::new();
    write_value(&mut out, value, style);
    out
}

fn write_value(out: &mut String, value: &Value, style: Style) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            // serde_json keeps integers that fit in 64 bits and reads every other
            // number as a correctly rounded float (`float_roundtrip`), like Python
            // except for integers beyond 64 bits, which GitHub never sends.
            let text = number.to_string();
            if text.contains(['.', 'e', 'E']) {
                out.push_str(&python_float(text.parse().unwrap_or(f64::NAN)));
            } else if text == "-0" {
                out.push('0');
            } else {
                out.push_str(&text);
            }
        }
        Value::String(text) => write_string(out, text, style.ensure_ascii),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(style.item_separator);
                }
                write_value(out, item, style);
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Sorted explicitly: serde_json's map order depends on crate features.
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push_str(style.item_separator);
                }
                write_string(out, key, style.ensure_ascii);
                out.push_str(style.key_separator);
                write_value(out, item, style);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str, ensure_ascii: bool) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            // Python escapes everything outside printable ASCII, DEL included.
            c if ensure_ascii && !(' '..='~').contains(&c) => {
                let mut units = [0_u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `repr(float)` de Python : chiffres les plus courts qui relisent la même valeur,
/// notation scientifique hors de `1e-4 <= |x| < 1e16`.
pub fn python_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // Rust's `{:e}` also prints the shortest round-tripping digits, e.g. "-1.2345e-7".
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let point = exponent + 1;
    let mut out = String::from(if negative { "-" } else { "" });
    if point <= -4 || point > 16 {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let shown = point - 1;
        let _ = write!(out, "e{}{:02}", if shown < 0 { '-' } else { '+' }, shown.unsigned_abs());
    } else if point <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat(point.unsigned_abs() as usize));
        out.push_str(&digits);
    } else {
        let point = point.unsigned_abs() as usize;
        if point >= digits.len() {
            out.push_str(&digits);
            out.push_str(&"0".repeat(point - digits.len()));
            out.push_str(".0");
        } else {
            out.push_str(&digits[..point]);
            out.push('.');
            out.push_str(&digits[point..]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn floats_match_python_repr() {
        let cases = [
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (9_999_999_999_999_998.0, "9999999999999998.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (1e100, "1e+100"),
            (123.456, "123.456"),
            (-2.5, "-2.5"),
            (100.0, "100.0"),
            (0.1, "0.1"),
            (-0.0, "-0.0"),
            (f64::MAX, "1.7976931348623157e+308"),
            (5e-324, "5e-324"),
        ];
        for (value, expected) in cases {
            assert_eq!(python_float(value), expected, "{value}");
        }
    }

    // Vectors produced with CPython 3 `json.dumps` and `hashlib.sha256`
    // (see the comment above each assertion for the exact Python call).
    #[test]
    fn compact_form_matches_python() {
        let payload = json!({
            "b": [1, 2.5, true, null],
            "a": {"z": "é\"\\\n\t\u{1}\u{7f}😀", "y": 1e16},
            "": -0.0001,
            "id": 9_007_199_254_740_993_u64
        });
        let compact = to_string(&payload, COMPACT);
        assert_eq!(
            compact,
            "{\"\":-0.0001,\"a\":{\"y\":1e+16,\"z\":\"é\\\"\\\\\\n\\t\\u0001\u{7f}😀\"},\
             \"b\":[1,2.5,true,null],\"id\":9007199254740993}"
        );
        // hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":"),
        //   ensure_ascii=False).encode()).hexdigest()
        assert_eq!(sha256_hex(&compact), "0921355b6d674894b6958d76d1f3603cdcc09ad55d7adaecd00ef5fd840f9f03");
    }

    #[test]
    fn numbers_are_read_as_python_reads_them() {
        // json.dumps(json.loads(text), sort_keys=True, separators=(",", ":"), ensure_ascii=False)
        let text = r#"{"f": 9.554674407370955e+19, "e": 1E5, "g": 0.1e1, "h": 2.50, "m": -1.5e-7, "i": 18446744073709551615}"#;
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            to_string(&value, COMPACT),
            r#"{"e":100000.0,"f":9.554674407370955e+19,"g":1.0,"h":2.5,"i":18446744073709551615,"m":-1.5e-07}"#
        );
    }

    #[test]
    fn known_differences_with_python() {
        // Python keeps an integer beyond 64 bits exact and reads `-0` as the int 0;
        // serde_json reads both as floats. GitHub sends neither.
        let value: Value = serde_json::from_str(r#"{"n": 95546744073709551616, "z": -0}"#).unwrap();
        assert_eq!(to_string(&value, COMPACT), r#"{"n":9.554674407370955e+19,"z":-0.0}"#);
    }

    #[test]
    fn serde_private_number_key_is_an_ordinary_key() {
        // With serde_json's `arbitrary_precision`, an object whose first key is this
        // marker is read back as a number: the canonical form, which sorts `$` first,
        // then stopped being valid JSON (found by fuzzing). The feature stays off.
        let text = r#"{"b":1,"$serde_json::private::Number":-1.0001}"#;
        let value: Value = serde_json::from_str(text).unwrap();
        let canonical = to_string(&value, COMPACT);
        assert_eq!(canonical, r#"{"$serde_json::private::Number":-1.0001,"b":1}"#);
        let again: Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(again, value);
        assert!(again.is_object());
    }

    #[test]
    fn python_default_form_escapes_non_ascii() {
        let manifest = json!({"version": 1, "files": {"users": "ab", "repos": "cd"}, "observed_date": null, "x": "é😀\u{7f}"});
        // Python escapes é, the surrogate pair of the emoji and DEL as \uXXXX.
        let escaped: String =
            ["00e9", "d83d", "de00", "007f"].iter().map(|unit| ["\\u", unit].concat()).collect();
        assert_eq!(
            to_string(&manifest, PYTHON_DEFAULT),
            format!(
                r#"{{"files": {{"repos": "cd", "users": "ab"}}, "observed_date": null, "version": 1, "x": "{escaped}"}}"#
            )
        );
    }

    #[test]
    fn evidence_hash_matches_python() {
        // hashlib.sha256(("https://api.github.com/repos/alice/example" + "\n" + json.dumps(REPO,
        //   sort_keys=True, separators=(",", ":"), ensure_ascii=False)).encode()).hexdigest()
        let repo = json!({"id": 123, "full_name": "alice/example", "private": false,
                          "owner": {"id": 42, "login": "alice", "type": "User"}});
        assert_eq!(
            evidence_hash("https://api.github.com/repos/alice/example", &repo),
            "42ffc53595fb56e724a78ad2fe5126df9a514f6ad2bf457be0a74b5648245a7a"
        );
    }
}
