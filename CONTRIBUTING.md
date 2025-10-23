# 🤝 Contributing to Thanks Stars

Thank you for your interest in improving Thanks Stars! This document describes how to build, test, and release the project so you can contribute effectively.

## 🧱 Prerequisites
- Rust toolchain (latest stable) with `cargo` and `rustfmt` installed via [rustup](https://rustup.rs/).
- `just` command runner for the bundled development tasks.
- Access to the GitHub REST API when exercising integration flows.

## 🗂 Project Structure and References
| Area | Purpose | Reference |
| --- | --- | --- |
| CLI entrypoint | Argument parsing and runtime orchestration | [`src/main.rs`](src/main.rs) |
| Core library | Discovery, GitHub client, and run loop | [`src/lib.rs`](src/lib.rs) |
| Configuration management | Token storage and environment overrides | [`src/config.rs`](src/config.rs) |
| GitHub integration | REST API wrapper and error handling | [`src/github.rs`](src/github.rs) |
| Dependency discovery | Aggregates framework support | [`src/discovery.rs`](src/discovery.rs) |
| Tests | End-to-end scenarios exercising the CLI | [`tests/`](tests) |
| Distribution config | Cargo Dist targets and installers | [`dist-workspace.toml`](dist-workspace.toml) |
| Release automation | GitHub Actions release pipeline | [`.github/workflows/release.yml`](.github/workflows/release.yml) |
| Crates.io publishing | Reusable publishing workflow | [`.github/workflows/publish-crates.yml`](.github/workflows/publish-crates.yml) |

## 🌐 Supported Ecosystems for Development
When extending ecosystem support, update the corresponding modules and fixtures below:

| Ecosystem | Detector / Parser | Key Module |
| --- | --- | --- |
| Cargo (Rust) | Parses `Cargo.lock` and `Cargo.toml` | [`src/ecosystems/cargo.rs`](src/ecosystems/cargo.rs) |
| Node.js (npm, Yarn, pnpm) | Reads `package.json` and lockfiles | [`src/ecosystems/node.rs`](src/ecosystems/node.rs) |
| Go (Go Modules) | Consumes `go.mod` files | [`src/ecosystems/go.rs`](src/ecosystems/go.rs) |

## 🛠 Development Workflow
Use the `just` recipes to keep formatting and linting consistent:

```bash
just fmt       # Format sources
just fmt-check # Verify formatting without writing changes
just lint      # Run Clippy with warnings as errors
just test      # Execute the test suite (pretty output)
just check     # Run all of the above in sequence
```

When introducing new dependencies or ecosystem features, add targeted tests under `tests/` that confirm repositories are detected correctly.

## 🧪 Running Tests Manually
- `cargo test` executes the Rust unit tests, including the configuration manager and discovery helpers.
- `cargo pretty-test` (via `just test`) runs the CLI integration scenarios with colorized diffs.

Ensure all tests pass locally before opening a pull request.

## 📦 Linux Packaging Contributions
We currently do not ship native packages for Debian/Ubuntu (`apt`/`apt-get`), Arch (`pacman`), or the Nix ecosystem. If you would like to help:

1. Propose the packaging layout in an issue so we can coordinate hosting and signing requirements.
2. Keep packaging scripts or manifests under a new `packaging/` directory (one subfolder per ecosystem) to avoid coupling them with the Rust build.
3. Mirror the release cadence defined in [`dist-workspace.toml`](dist-workspace.toml) so GitHub Releases remain the source of truth.
4. Document the installation steps in [`README.md`](README.md) once the publishing infrastructure is ready.

Feel free to link to existing community overlays (e.g., Nix flakes or AUR recipes) while work toward official packages is underway.

## 🚢 Release Process
Releases are automated and happen when you push a semver-style tag (for example `v1.2.3`). The [`release.yml`](.github/workflows/release.yml) workflow will:
1. Generate distribution artifacts for Linux, macOS, and Windows targets defined in [`dist-workspace.toml`](dist-workspace.toml).
2. Publish installers (shell, PowerShell, Homebrew) and upload the assets to the GitHub Release.
3. Update the Homebrew tap [`Kenzo-Wada/thanks-stars`](https://github.com/Kenzo-Wada/homebrew-thanks-stars).
4. Trigger [`publish-crates.yml`](.github/workflows/publish-crates.yml) to release the crate to crates.io once the GitHub Release succeeds.

Coordinate with the maintainers to ensure secrets such as `CARGO_REGISTRY_TOKEN` are available before cutting a release.

## 💬 Communication
Open issues or discussions on GitHub to propose significant changes. When submitting a pull request, describe the motivation, testing performed, and any follow-up work that might be required.
