use anyhow::{Context, Result, anyhow, bail};
use dirs::home_dir;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use walkdir::WalkDir;

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
    let static_dir = root.join("frontend/static");
    if !static_dir.exists() {
        bail!(
            "expected static assets at {} – ensure frontend/static exists",
            static_dir.display()
        );
    }

    let dist_dir = root.join("frontend/dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir)
            .with_context(|| format!("failed to clear {}", dist_dir.display()))?;
    }
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;

    copy_directory(&static_dir, &dist_dir)?;

    if matches!(profile, BuildProfile::Release) {
        println!("Frontend assets copied (release profile).");
    } else {
        println!("Frontend assets copied (debug profile).");
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

fn copy_directory(from: &Path, to: &Path) -> Result<()> {
    for entry in WalkDir::new(from) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(from).expect("strip prefix");
        let dest_path = to.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest_path)
                .with_context(|| format!("failed to create directory {}", dest_path.display()))?;
        } else {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(entry.path(), &dest_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry.path().display(),
                    dest_path.display()
                )
            })?;
        }
    }
    Ok(())
}
