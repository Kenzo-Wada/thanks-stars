# 🌟 Thanks Stars

Thanks Stars is a command-line companion that stars the GitHub repositories powering your project so you can show appreciation to the maintainers of your dependencies. It draws inspiration from [teppeis/thank-you-stars](https://github.com/teppeis/thank-you-stars), but reimagines the experience in Rust with first-class support for multiple ecosystems.

https://github.com/user-attachments/assets/d7a7b047-312e-4d56-ba5d-25ed6eb2e5ce

## ✨ Highlights

- Detects dependencies across popular package managers and build tools.
- Uses your GitHub personal access token to star repositories on your behalf.
- Provides friendly progress output and summarizes what happened at the end of a run.

## 🧭 Supported Ecosystems

The following ecosystems are currently detected when you run the tool:

| Ecosystem       | Detection Source                                                               | Implementation                                             |
| --------------- | ------------------------------------------------------------------------------ | ---------------------------------------------------------- |
| Cargo (Rust)    | `Cargo.toml`                                                                   | [`src/ecosystems/cargo.rs`](src/ecosystems/cargo.rs)       |
| Node.js         | `package.json`                                                                 | [`src/ecosystems/node.rs`](src/ecosystems/node.rs)         |
| Go (Go Modules) | `go.mod`                                                                       | [`src/ecosystems/go.rs`](src/ecosystems/go.rs)             |
| Composer (PHP)  | `composer.lock` / `composer.json`                                              | [`src/ecosystems/composer.rs`](src/ecosystems/composer.rs) |
| Ruby (Bundler)  | `Gemfile` / `Gemfile.lock`                                                     | [`src/ecosystems/ruby.rs`](src/ecosystems/ruby.rs)         |
| Python          | `pyproject.toml` / `requirements.txt` / `Pipfile` / `Pipfile.lock` / `uv.lock` | [`src/ecosystems/python.rs`](src/ecosystems/python.rs)     |

Looking for support for a different ecosystem? [Open an ecosystem support request](https://github.com/Kenzo-Wada/thanks-stars/issues/new?template=ecosystem_support_request.md) and tell us about the manifest and lockfiles we should detect.

## 🚀 Installation

Choose the installation method that best fits your environment:

### 🍺 Homebrew

```bash
$ brew tap Kenzo-Wada/thanks-stars
$ brew install thanks-stars
```

### 🦀 Cargo

```bash
$ cargo install thanks-stars
```

### 💻 Shell installer (macOS/Linux)

```bash
$ curl -LSfs https://github.com/Kenzo-Wada/thanks-stars/releases/latest/download/thanks-stars-installer.sh | sh
```

### 🪟 PowerShell installer (Windows)

```powershell
$ iwr https://github.com/Kenzo-Wada/thanks-stars/releases/latest/download/thanks-stars-installer.ps1 -useb | iex
```

### 🐧 Linux package managers

Native packages like `apt`/`apt-get`, `pacman`, `nix`, etc... are not published yet. Until a maintainer volunteers to host those repositories, please use one of the installers above or the manual download method. Contributions toward official Linux packages are very welcome—see [CONTRIBUTING.md](CONTRIBUTING.md) for coordination details.

### 📦 Manual download

Grab the archive for your platform from the [GitHub Releases](https://github.com/Kenzo-Wada/thanks-stars/releases) page and place the `thanks-stars` binary somewhere on your `PATH`.

## 🛠 Usage

Authenticate once with a GitHub personal access token, then run the tool in the root of your project.

### Authenticate with GitHub

```bash
$ thanks-stars auth --token ghp_your_token_here
```

If you omit `--token`, the command will prompt you to paste it securely. By default the configuration is stored in a user-specific `config.toml`; you can override the storage location with the `THANKS_STARS_CONFIG_DIR` environment variable.

### Run inside your project

```bash
$ cd path/to/your/project
$ thanks-stars
```

Example output:

```
$ thanks-stars
⭐ Starred https://github.com/xxx/xxx via Cargo.toml
⭐ Starred https://github.com/xxx/xxx via package.json
...
✨ Completed! Starred 10 repositories.
```

Run `thanks-stars --help` to explore all available options.

#### Preview your run with `--dry-run`

If you want to see which repositories would be starred without making any
changes to your GitHub account, pass the `--dry-run` flag:

```
$ thanks-stars --dry-run
⭐ Would star https://github.com/xxx/xxx via Cargo.toml
⭐ Already starred https://github.com/xxx/yyy via package.json
✨ Dry run complete! 1 repository would be starred, 1 already starred.
```

The command still inspects your dependencies so it can report which ones are
already starred, but it avoids sending any API requests that would change your
starred repositories.

---

Thanks Stars helps you recognize the maintainers who keep your stack running—so while you're at it, don't forget to ⭐ this project too!
