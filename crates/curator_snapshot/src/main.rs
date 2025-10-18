use clap::Parser;

use curator_snapshot::config::CliArgs;

#[tokio::main]
async fn main() {
    let cli = CliArgs::parse();
    if let Err(err) = curator_snapshot::run(cli).await {
        eprintln!("curator snapshot failed: {}", err);
        std::process::exit(1);
    }
}
