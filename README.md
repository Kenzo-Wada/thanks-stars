# 🌟 Thanks Stars

Thanks Stars is a command-line companion that stars the GitHub repositories powering your project so you can show appreciation to the maintainers of your dependencies. It draws inspiration from [teppeis/thank-you-stars](https://github.com/teppeis/thank-you-stars), but reimagines the experience in Rust with first-class support for multiple ecosystems.

## ✨ Highlights
- Detects dependencies across popular package managers and build tools.
- Uses your GitHub personal access token to star repositories on your behalf.
- Provides friendly progress output and summarizes what happened at the end of a run.

## 🧭 Supported Ecosystems
The following ecosystems are currently detected when you run the tool:

| Ecosystem | Detection Source | Implementation |
| --- | --- | --- |
| Cargo (Rust) | `Cargo.lock` / `Cargo.toml` | [`src/ecosystems/cargo.rs`](src/ecosystems/cargo.rs) |
| Node.js (npm, Yarn, pnpm) | `package.json` with lockfiles (`package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`) | [`src/ecosystems/node.rs`](src/ecosystems/node.rs) |
| Gradle (Java/Kotlin) | `build.gradle` / `build.gradle.kts` | [`src/ecosystems/gradle.rs`](src/ecosystems/gradle.rs) |
| Go (Go Modules) | `go.mod` | [`src/ecosystems/go.rs`](src/ecosystems/go.rs) |
| Python (pip, requirements, uv) | `requirements.txt` / `uv.lock` | [`src/ecosystems/python.rs`](src/ecosystems/python.rs) |
| Ruby (Bundler) | `Gemfile.lock` | [`src/ecosystems/ruby.rs`](src/ecosystems/ruby.rs) |
| Deno | `deno.json` / `deno.jsonc` | [`src/ecosystems/deno.rs`](src/ecosystems/deno.rs) |
| JSR | `jsr.json` | [`src/ecosystems/jsr.rs`](src/ecosystems/jsr.rs) |

## 🚀 Installation
Choose the installation method that best fits your environment:

### 🍺 Homebrew
```bash
brew tap Kenzo-Wada/thanks-stars
brew install thanks-stars
```

### 🦀 Cargo
```bash
cargo install thanks-stars
```

### 💻 Shell installer (macOS/Linux)
```bash
curl -LSfs https://github.com/Kenzo-Wada/thanks-stars/releases/latest/download/thanks-stars-installer.sh | sh
```

### 🪟 PowerShell installer (Windows)
```powershell
iwr https://github.com/Kenzo-Wada/thanks-stars/releases/latest/download/thanks-stars-installer.ps1 -useb | iex
```

### 🐧 Linux package managers
Native packages for `apt`/`apt-get`, `pacman`, and Nix are not published yet. Until a maintainer volunteers to host those repositories, please use one of the installers above or the manual download method. Contributions toward official Linux packages are very welcome—see [CONTRIBUTING.md](CONTRIBUTING.md) for coordination details.

### 📦 Manual download
Grab the archive for your platform from the [GitHub Releases](https://github.com/Kenzo-Wada/thanks-stars/releases) page and place the `thanks-stars` binary somewhere on your `PATH`.

## 🛠 Usage
Authenticate once with a GitHub personal access token, then run the tool in the root of your project:

```bash
thanks-stars auth --token ghp_your_token_here
thanks-stars run
```

If you omit `--token`, the command will prompt you to paste it securely. By default the configuration is stored in a user-specific `config.toml`; you can override the storage location with the `THANKS_STARS_CONFIG_DIR` environment variable.

Run `thanks-stars --help` to explore all available options.

## ⭐ Show Some Love
Thanks Stars helps you recognize the maintainers who keep your stack running—so while you're at it, don't forget to ⭐ this project too!
