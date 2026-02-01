//! Main entry point for rconv
//!
//! This binary supports both CLI and GUI modes:
//! - CLI mode: When sufficient arguments are provided for command-line execution
//! - GUI mode: When no arguments or insufficient arguments are provided

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Determine whether to run in CLI or GUI mode
    if rconv_cli::should_run_cli_mode() {
        // CLI mode
        rconv_cli::run().await.map_err(|e| anyhow::anyhow!(e))?;
    } else {
        // GUI mode
        if let Err(e) = rconv_gui::run() {
            eprintln!("GUI error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
