use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

use crate::Result;
use crate::error::Error;

/// Initialise tracing avec un filtre optionnel et un mode JSON conditionnel.
///
/// # Errors
///
/// Retourne une erreur si le filtre fourni est invalide, si la couche JSON est
/// demandée alors que la fonctionnalité n'est pas compilée, ou si l'installation
/// du subscriber global échoue.
pub fn init_tracing(explicit_filter: Option<&str>, use_json: bool) -> Result<()> {
    let mut filter_candidates = Vec::new();
    if let Some(f) = explicit_filter {
        filter_candidates.push(f.to_string());
    }
    if let Ok(env) = std::env::var("RUST_LOG") {
        filter_candidates.push(env);
    }
    filter_candidates.push("info".to_string());

    let filter = filter_candidates
        .into_iter()
        .find_map(|candidate| EnvFilter::try_new(candidate).ok())
        .ok_or_else(|| Error::Telemetry("invalid log filter".to_string()))?;

    #[cfg(feature = "json-logs")]
    if use_json {
        let subscriber = Registry::default().with(filter).with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_file(true)
                .with_line_number(true)
                .json()
                .flatten_event(true),
        );
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|err| Error::Telemetry(err.to_string()))?;
        return Ok(());
    }

    #[cfg(not(feature = "json-logs"))]
    if use_json {
        return Err(Error::Telemetry(
            "binary was built without the `json-logs` feature".to_string(),
        ));
    }

    let subscriber = Registry::default().with(filter).with(
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_file(true)
            .with_line_number(true),
    );
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|err| Error::Telemetry(err.to_string()))
}
