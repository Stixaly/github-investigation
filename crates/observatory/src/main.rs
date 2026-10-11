//! `observatory` : API, worker et administration locale.

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use observatory::api::{self, AppState};
use observatory::collector::GitHub;
use observatory::config::{ApiConfig, secret, var_or};
use observatory::db::{DbSettings, pool, tune};
use observatory::legacy::{ImportRequest, import_snapshot};
use observatory::{demo, migrate, users, worker};
use sqlx::Connection;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "observatory",
    version,
    about = "Investigation des relations entre comptes et dépôts GitHub publics"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Sert l'API et l'interface compilée (rôle applicatif PostgreSQL).
    Serve,
    /// Exécute les scans GitHub en file (rôle applicatif PostgreSQL).
    Worker,
    /// Applique les migrations et les droits du rôle applicatif (rôle administrateur PostgreSQL).
    Migrate {
        /// Vérifie seulement que le schéma est à jour.
        #[arg(long)]
        check: bool,
    },
    /// Crée un compte analyste local ; le mot de passe est demandé sans écho.
    CreateUser {
        login: String,
        /// Lit le mot de passe sur l'entrée standard (une ligne).
        #[arg(long)]
        password_stdin: bool,
    },
    /// Crée un dossier de démonstration aux données fictives.
    Demo {
        #[arg(long)]
        login: String,
    },
    /// Importe un ancien instantané users_db.json / repos_db.json.
    ImportLegacy {
        directory: PathBuf,
        #[arg(long)]
        investigation: Uuid,
        #[arg(long)]
        login: String,
        /// Date d'observation (AAAA-MM-JJ) ; omettre si inconnue.
        #[arg(long)]
        observed_date: Option<NaiveDate>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));
    if var_or("LOG_FORMAT", "text") == "json" {
        tracing_subscriber::fmt().json().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    // The containers cap pids and CPU: keep the thread pools small and explicit.
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .max_blocking_threads(16)
        .enable_all()
        .build()
        .expect("tokio runtime");
    match runtime.block_on(run(cli.command)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = interrupt => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = interrupt.await;
    tracing::info!("shutdown requested");
}

#[allow(clippy::print_stdout)]
async fn run(command: Command) -> anyhow::Result<()> {
    let settings = DbSettings::from_env()?;
    match command {
        Command::Serve => {
            let config = ApiConfig::from_env()?;
            let pool = pool(settings.options(), 10).await.context("connexion PostgreSQL")?;
            migrate::verify(&pool).await?;
            api::serve(AppState { pool, config: Arc::new(config) }, shutdown_signal()).await
        }
        Command::Worker => {
            let pool = pool(settings.options(), 4).await.context("connexion PostgreSQL")?;
            migrate::verify(&pool).await?;
            let github = GitHub::new(&secret("GITHUB_TOKEN")?)?;
            tracing::info!("worker started");
            worker::run(pool, github, Duration::from_secs(1), shutdown_signal()).await;
            Ok(())
        }
        Command::Migrate { check } => {
            let options = tune(settings.options(), Duration::ZERO);
            let mut conn =
                sqlx::PgConnection::connect_with(&options).await.context("connexion PostgreSQL")?;
            if check {
                let pool = pool(settings.options(), 1).await?;
                migrate::verify(&pool).await?;
                println!("Schéma à jour (version {})", migrate::latest());
                return Ok(());
            }
            let password = secret("APP_PASSWORD")?;
            let applied = migrate::migrate(&mut conn, Some(&password), None).await?;
            println!("Schéma en version {} ; migrations appliquées : {applied:?}", migrate::latest());
            Ok(())
        }
        Command::CreateUser { login, password_stdin } => {
            let password = if password_stdin || !std::io::stdin().is_terminal() {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                line.trim_end_matches(['\r', '\n']).to_owned()
            } else {
                rpassword::prompt_password("Mot de passe (14 caractères minimum) : ")?
            };
            let pool = pool(settings.options(), 1).await?;
            users::create_user(&pool, &login, &password).await?;
            println!("Compte créé");
            Ok(())
        }
        Command::Demo { login } => {
            let pool = pool(settings.options(), 1).await?;
            println!("{}", demo::create_demo(&pool, &login).await?);
            Ok(())
        }
        Command::ImportLegacy { directory, investigation, login, observed_date } => {
            let pool = pool(settings.options(), 2).await?;
            let request = ImportRequest { directory, investigation, login, observed_date };
            let report = import_snapshot(&pool, &request).await?;
            println!("{}", serde_json::to_string(&report)?);
            Ok(())
        }
    }
}
