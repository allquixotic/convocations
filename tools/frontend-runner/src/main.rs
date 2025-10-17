use anyhow::{Context, Result, anyhow, bail};
use dirs::home_dir;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let action = args.next().unwrap_or_else(|| "help".to_string());

    match action.as_str() {
        "build-release" => build_frontend(BuildProfile::Release),
        "build-debug" => build_frontend(BuildProfile::Debug),
        "gui-build" => tauri_build(false),
        "gui-build-debug" => tauri_build(true),
        "gui-dev" => tauri_dev(),
        "cli-release" => cli_release(),
        "cli-deploy" => cli_deploy(),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => bail!("unknown frontend-runner action: {other}"),
    }
}

fn print_help() {
    println!(
        "frontend-runner usage:
  cargo run -p frontend-runner -- build-release      # build frontend assets (release)
  cargo run -p frontend-runner -- build-debug        # build frontend assets (debug)
  cargo run -p frontend-runner -- gui-build          # bundle the GUI release
  cargo run -p frontend-runner -- gui-build-debug    # bundle the GUI with Rust debug profile
  cargo run -p frontend-runner -- gui-dev            # start Tauri dev (rebuilds frontend first)
  cargo run -p frontend-runner -- cli-release        # build CLI release binary
  cargo run -p frontend-runner -- cli-deploy         # build + copy CLI to ~/.bin/convocations_bin"
    );
}

fn build_frontend(profile: BuildProfile) -> Result<()> {
    let root = workspace_root();
    let frontend_dir = root.join("frontend");

    if !frontend_dir.exists() {
        bail!(
            "expected frontend directory at {} – ensure frontend/ exists",
            frontend_dir.display()
        );
    }

    // Run bun build via the build script
    let mut command = Command::new("bun");
    command
        .current_dir(&frontend_dir)
        .args(["run", "build.js"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if matches!(profile, BuildProfile::Release) {
        command.arg("--release");
    }

    run(&mut command)?;

    if matches!(profile, BuildProfile::Release) {
        println!("Frontend built with Bun (release profile).");
    } else {
        println!("Frontend built with Bun (debug profile).");
    }

    Ok(())
}

fn cli_release() -> Result<()> {
    let root = workspace_root();
    let mut command = Command::new("cargo");
    command
        .current_dir(&root)
        .args(["build", "--release", "-p", "rconv"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    run(&mut command)
}

fn cli_deploy() -> Result<()> {
    cli_release()?;

    let root = workspace_root();
    let binary = root.join("target/release/rconv");
    if !binary.exists() {
        bail!(
            "expected CLI binary at {} – run `cargo cli-release` first",
            binary.display()
        );
    }

    let home = home_dir().context("failed to locate home directory")?;
    let dest_dir = home.join(".bin");
    fs::create_dir_all(&dest_dir)
        .with_context(|| format!("failed to create {}", dest_dir.display()))?;

    let dest_path = dest_dir.join("convocations_bin");
    fs::copy(&binary, &dest_path).with_context(|| {
        format!(
            "failed to copy {} to {}",
            binary.display(),
            dest_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&dest_path, perms).with_context(|| {
            format!(
                "failed to set executable permissions on {}",
                dest_path.display()
            )
        })?;
    }

    println!("Deployed CLI binary to {}", dest_path.display());

    Ok(())
}

fn tauri_build(debug: bool) -> Result<()> {
    let root = workspace_root();
    let mut command = Command::new("cargo");
    command
        .current_dir(&root)
        .args(["tauri", "build"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if debug {
        command.arg("--debug");
    }

    run(&mut command)
}

fn tauri_dev() -> Result<()> {
    let root = workspace_root();
    let mut command = Command::new("cargo");
    command
        .current_dir(&root)
        .args(["tauri", "dev"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    run(&mut command)
}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to spawn command {:?}", command.get_program()))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "command {:?} exited with {}",
            command.get_program(),
            status
        ))
    }
}

#[derive(Clone, Copy)]
enum BuildProfile {
    Debug,
    Release,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tools/frontend-runner parent")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

