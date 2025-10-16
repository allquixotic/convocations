# GitHub Actions CI/CD

This directory contains GitHub Actions workflows for automated building and releasing of the convocations tool.

## Workflows

### `build.yml` - Build and Release

This workflow handles:
- **Continuous Integration**: Builds on every push and pull request to main/master
- **Release Automation**: Creates GitHub releases with binaries when tags are pushed

#### Platforms

The workflow builds for three platforms:

1. **Linux x86-64** (`ubuntu-24.04`)
   - Target: `x86_64-unknown-linux-gnu`
   - Output: `rconv-linux-x86_64.tar.gz`

2. **Windows x86-64** (`windows-latest` - Windows Server 2022)
   - Target: `x86_64-pc-windows-msvc`
   - Output: `rconv-windows-x86_64.zip`

3. **MacOS Universal** (`macos-14` - MacOS Sonoma)
   - Targets: `aarch64-apple-darwin` + `x86_64-apple-darwin`
   - Combined with `lipo` into a universal binary
   - Output: `rconv-macos-universal.tar.gz`

#### Rust Version

The workflow uses Rust 1.89.0 as specified in the project requirements.

#### Triggers

- **Push to main/master**: Builds all platforms but doesn't create a release
- **Push a tag starting with 'v'**: Builds all platforms and creates a GitHub release
- **Pull Request**: Builds all platforms for validation
- **Manual dispatch**: Can be triggered manually from GitHub Actions UI

## Creating a Release

To create a new release with binaries:

1. **Create and push a version tag**:
   ```bash
   git tag v1.0.0
   git push origin v1.0.0
   ```

2. **GitHub Actions will automatically**:
   - Build binaries for all three platforms
   - Create compressed archives (tar.gz for Linux/MacOS, zip for Windows)
   - Create a GitHub release with the tag
   - Upload all binary archives to the release
   - Generate release notes from commit history

3. **Users can then download**:
   - `rconv-linux-x86_64.tar.gz` - Linux binary
   - `rconv-windows-x86_64.zip` - Windows executable
   - `rconv-macos-universal.tar.gz` - MacOS universal binary (works on both Intel and Apple Silicon)

## Cache Strategy

The workflow caches:
- Cargo registry (`~/.cargo/registry`)
- Cargo git index (`~/.cargo/git`)
- Build artifacts (`target/`)

This significantly speeds up subsequent builds by avoiding re-downloading dependencies and re-compiling unchanged code.

## Binary Optimization

- All binaries are built with `--release` for maximum performance
- Linux and MacOS binaries are stripped to reduce file size
- Universal MacOS binary includes both ARM64 and x86-64 architectures

## Testing Locally

To test that your code builds for all platforms without pushing:

```bash
# Linux (if on Linux)
cargo build --release --target x86_64-unknown-linux-gnu

# Windows (if on Windows)
cargo build --release --target x86_64-pc-windows-msvc

# MacOS Universal (if on MacOS)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create \
  target/aarch64-apple-darwin/release/rconv \
  target/x86_64-apple-darwin/release/rconv \
  -output rconv-universal
```

## Troubleshooting

### Build Fails on a Platform

- Check the Actions tab in your GitHub repository
- Click on the failed workflow run
- Expand the failed step to see error messages
- Common issues:
  - Missing dependencies in Cargo.toml
  - Platform-specific code that doesn't compile on all targets
  - Rust version incompatibility

### Release Not Created

- Ensure the tag starts with 'v' (e.g., `v1.0.0`, not `1.0.0`)
- Check that you pushed the tag: `git push origin v1.0.0`
- Verify repository permissions allow creating releases
- Check the Actions tab for error messages

### Artifacts Missing

- Ensure all three build jobs completed successfully
- Check the "Upload artifact" step in each build job
- Verify the binary names match what's expected in the workflow
