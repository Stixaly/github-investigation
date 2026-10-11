//! Configuration lue dans l'environnement ; les secrets viennent de fichiers.

use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, bail};

/// Valeur d'un secret : contenu du fichier `<NAME>_FILE` s'il est défini, sinon `<NAME>`.
///
/// Compose monte les secrets en fichiers ; la variable directe ne sert qu'aux
/// tests et aux bases jetables.
pub fn secret(name: &str) -> anyhow::Result<String> {
    match env::var(format!("{name}_FILE")) {
        Ok(path) if !path.is_empty() => Ok(fs::read_to_string(&path)
            .with_context(|| format!("lecture du secret {name}_FILE"))?
            .trim()
            .to_owned()),
        _ => Ok(env::var(name).unwrap_or_default()),
    }
}

pub fn var_or(name: &str, default: &str) -> String {
    env::var(name).ok().filter(|value| !value.is_empty()).unwrap_or_else(|| default.to_owned())
}

pub fn flag(name: &str) -> bool {
    matches!(env::var(name).as_deref(), Ok("1" | "true" | "yes"))
}

/// Origine publique de l'interface, seule autorisée à envoyer des requêtes modifiantes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// `https://observatory.example`, sans barre finale.
    pub origin: String,
    pub host: String,
    pub secure: bool,
}

impl Origin {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        let origin = value.trim_end_matches('/');
        let Some((scheme, authority)) = origin.split_once("://") else {
            bail!("PUBLIC_ORIGIN must be an origin, without path");
        };
        if !matches!(scheme, "http" | "https")
            || authority.is_empty()
            || authority.contains(['/', '?', '#', '@'])
        {
            bail!("PUBLIC_ORIGIN must be an origin, without path");
        }
        let host = match authority.strip_prefix('[') {
            Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
            None => authority.split(':').next().unwrap_or_default(),
        }
        .to_ascii_lowercase();
        if host.is_empty() {
            bail!("PUBLIC_ORIGIN must be an origin, without path");
        }
        let secure = scheme == "https";
        if !secure && !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
            bail!("HTTPS is mandatory except on loopback");
        }
        Ok(Self { origin: origin.to_owned(), host, secure })
    }
}

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub origin: Origin,
    /// Lire l'adresse du client dans `X-Real-IP` (posé par le proxy HTTPS).
    pub trust_proxy_headers: bool,
    pub static_dir: PathBuf,
    pub listen: SocketAddr,
}

impl ApiConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            origin: Origin::parse(&var_or("PUBLIC_ORIGIN", "http://localhost:8088"))?,
            trust_proxy_headers: flag("TRUST_PROXY_HEADERS"),
            static_dir: PathBuf::from(var_or("STATIC_DIR", "static")),
            listen: var_or("LISTEN", "0.0.0.0:8000").parse().context("LISTEN")?,
        })
    }

    /// Hôtes acceptés dans l'en-tête `Host`.
    pub fn allowed_hosts(&self) -> [&str; 4] {
        [self.origin.host.as_str(), "localhost", "127.0.0.1", "api"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_validated() {
        let local = Origin::parse("http://localhost:8088/").unwrap();
        assert_eq!(
            (local.origin.as_str(), local.host.as_str(), local.secure),
            ("http://localhost:8088", "localhost", false)
        );
        assert_eq!(Origin::parse("https://Obs.Example:8443").unwrap().host, "obs.example");
        assert_eq!(Origin::parse("http://[::1]:8088").unwrap().host, "::1");
        for bad in
            ["http://team.example", "https://x.example/path", "ftp://localhost", "localhost:8088", "https://"]
        {
            assert!(Origin::parse(bad).is_err(), "{bad}");
        }
    }
}
