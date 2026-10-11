//! Lecture en flux des anciens instantanés `users_db.json` et `repos_db.json`.
//!
//! Les fichiers peuvent dépasser 300 Mo : un dépôt est remis à l'appelant dès
//! qu'il est lu, sans charger la liste complète d'un compte. Le JSON est
//! traité comme une donnée inerte ; la profondeur d'imbrication est bornée par
//! serde_json.

use std::fmt;
use std::io::Read;

use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

/// Parcourt `{"login": {...}, ...}` et appelle `on_user` pour chaque entrée.
///
/// `on_user` peut interrompre la lecture en renvoyant une erreur.
pub fn for_each_user<R: Read>(
    reader: R,
    on_user: impl FnMut(String, Value) -> Result<(), String>,
) -> serde_json::Result<()> {
    struct Users<F>(F);
    impl<'de, F: FnMut(String, Value) -> Result<(), String>> Visitor<'de> for Users<F> {
        type Value = ();
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an object of users")
        }
        fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<(), A::Error> {
            while let Some(name) = map.next_key::<String>()? {
                let user: Value = map.next_value()?;
                (self.0)(name, user).map_err(de::Error::custom)?;
            }
            Ok(())
        }
    }
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    deserializer.deserialize_map(Users(on_user))?;
    deserializer.end()
}

/// Parcourt `{"login": [{dépôt}, ...], ...}` dépôt par dépôt.
///
/// Une entrée qui n'est pas une liste (`{"error": "404"}`) et un élément qui
/// n'est pas un objet sont ignorés.
pub fn for_each_repository<R: Read>(
    reader: R,
    mut on_repository: impl FnMut(&str, Value) -> Result<(), String>,
) -> serde_json::Result<()> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    deserializer.deserialize_map(Owners(&mut on_repository))?;
    deserializer.end()
}

struct Owners<'f, F>(&'f mut F);

impl<'de, F: FnMut(&str, Value) -> Result<(), String>> Visitor<'de> for Owners<'_, F> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an object of repository lists")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(owner) = map.next_key::<String>()? {
            map.next_value_seed(Repositories { owner: &owner, on_repository: &mut *self.0 })?;
        }
        Ok(())
    }
}

struct Repositories<'a, F> {
    owner: &'a str,
    on_repository: &'a mut F,
}

impl<'de, F: FnMut(&str, Value) -> Result<(), String>> DeserializeSeed<'de> for Repositories<'_, F> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de, F: FnMut(&str, Value) -> Result<(), String>> Visitor<'de> for Repositories<'_, F> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a list of repositories")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(item) = seq.next_element::<Value>()? {
            if item.is_object() {
                (self.on_repository)(self.owner, item).map_err(de::Error::custom)?;
            }
        }
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(())
    }
    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_repositories_and_skips_errors() {
        let input = br#"{"alice":[{"full_name":"alice/a","topics":["x"]},3,{"full_name":"alice/b"}],"bad":{"error":"404"},"none":null}"#;
        let mut seen = Vec::new();
        for_each_repository(&input[..], |owner, repository| {
            seen.push((owner.to_owned(), repository["full_name"].as_str().unwrap().to_owned()));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, [("alice".into(), "alice/a".into()), ("alice".into(), "alice/b".into())]);
    }

    #[test]
    fn streams_users() {
        let mut seen = Vec::new();
        for_each_user(&br#"{"alice":{"id":1},"bob":{"error":"404"}}"#[..], |name, user| {
            seen.push((name, user));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn stops_when_the_consumer_fails() {
        let error =
            for_each_repository(&br#"{"a":[{"x":1},{"x":2}]}"#[..], |_, _| Err("stop".into())).unwrap_err();
        assert!(error.to_string().contains("stop"));
    }

    #[test]
    fn malformed_files_are_errors() {
        assert!(for_each_repository(&b"[1, 2]"[..], |_, _| Ok(())).is_err());
        assert!(for_each_repository(&br#"{"a":[{"x":1}"#[..], |_, _| Ok(())).is_err());
        assert!(for_each_user(&br#"{"a":{}} trailing"#[..], |_, _| Ok(())).is_err());
        let deep = format!("{{\"a\":[{}{}]}}", "[".repeat(10_000), "]".repeat(10_000));
        assert!(for_each_repository(deep.as_bytes(), |_, _| Ok(())).is_err(), "nesting is bounded");
    }
}
