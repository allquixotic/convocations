//! CLI binary entry point for rconv-cli
//!
//! This is a thin wrapper around the library functions.
//! The main binary (`rconv`) will have mode detection logic.

#[tokio::main]
async fn main() {
    if let Err(err) = rconv_cli::run().await {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
