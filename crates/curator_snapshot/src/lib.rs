pub mod alias;
pub mod config;
pub mod curate;
pub mod error;
pub mod fetch;
pub mod snapshot;

use reqwest::Client;

use alias::load_alias_map;
use config::{AppConfig, CliArgs};
use curate::curate_models;
use error::CuratorError;
use fetch::fetch_datasets;
use snapshot::{materialize_snapshot, write_snapshot};

pub async fn run(cli: CliArgs) -> Result<(), CuratorError> {
    let AppConfig { paths, tunables } = cli.resolve()?;

    let client = Client::builder()
        .user_agent("rconv-curator-snapshot/0.1")
        .build()?;

    let aliases = load_alias_map(&paths.aliases)?;
    let datasets = fetch_datasets(&client, &tunables).await?;
    let computation = curate_models(aliases, &datasets.openrouter, &datasets.aa, &tunables);

    let snapshot = materialize_snapshot(computation, &tunables);
    write_snapshot(&paths.snapshot, &snapshot)?;

    println!(
        "Snapshot written to {} (schema v{})",
        paths.snapshot.display(),
        snapshot.schema_version
    );

    Ok(())
}
