use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use owo_colors::OwoColorize;
use supports_color::Stream as ColorStream;

use thanks_stars::config::{ConfigError, ConfigManager};
use thanks_stars::discovery::Repository;
use thanks_stars::github::{GitHubApi, GitHubClient, GitHubError};
use thanks_stars::{run_with_handler, RunError, RunEventHandler, RunSummary};

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Star the GitHub repositories of your dependencies."
)]
struct Cli {
    #[command(flatten)]
    run: RunArgs,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Configure the GitHub personal access token used for starring repositories.
    Auth(AuthArgs),
    /// Star dependencies for the current project.
    Run(RunArgs),
}

#[derive(Args, Default)]
struct AuthArgs {
    /// GitHub personal access token (if omitted, you will be prompted).
    #[arg(long)]
    token: Option<String>,
}

#[derive(Args, Default, Clone)]
struct RunArgs {
    /// Path to the project root. Defaults to the current directory.
    #[arg(short, long)]
    path: Option<PathBuf>,
    /// Simulate starring repositories without issuing star requests to GitHub.
    #[arg(long = "dry-run")]
    dry_run: bool,
}

fn main() -> Result<()> {
    let Cli { run, command } = Cli::parse();
    let config = ConfigManager::new()?;

    match command {
        Some(Commands::Auth(args)) => handle_auth(args, &config),
        Some(Commands::Run(args)) => handle_run(args, &config),
        None => handle_run(run, &config),
    }
}

fn handle_auth(args: AuthArgs, config: &ConfigManager) -> Result<()> {
    let token = match args.token {
        Some(token) if !token.trim().is_empty() => token,
        _ => prompt_for_token()?,
    };

    config
        .save_token(token.trim())
        .context("failed to save GitHub token")?;
    println!("Token saved to {}", config.config_file().display());
    Ok(())
}

fn handle_run(args: RunArgs, config: &ConfigManager) -> Result<()> {
    let root = args
        .path
        .unwrap_or(std::env::current_dir().context("failed to determine current directory")?);

    let token = load_token(config)?;
    let client = create_client(token).context("failed to initialize GitHub client")?;

    let mut handler = CliRunHandler::new(args.dry_run);
    let adapter = MaybeDryRunClient::new(&client, args.dry_run);
    run_with_handler(&root, &adapter, &mut handler).map_err(|err| match err {
        RunError::NoFrameworks(path) => {
            anyhow!("no supported dependency definitions found in {path}")
        }
        RunError::Discovery(inner) => anyhow!(inner),
        RunError::GitHub(inner) => anyhow!(inner),
    })?;
    Ok(())
}

fn create_client(token: String) -> Result<GitHubClient, GitHubError> {
    if let Ok(base) = std::env::var("THANKS_STARS_API_BASE") {
        GitHubClient::with_base_url(token, base)
    } else {
        GitHubClient::new(token)
    }
}

fn prompt_for_token() -> Result<String> {
    print!("GitHub personal access token: ");
    io::stdout().flush().ok();
    let mut token = String::new();
    io::stdin()
        .read_line(&mut token)
        .context("failed to read token from stdin")?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("token must not be empty"));
    }
    Ok(token)
}

struct CliRunHandler {
    progress: Option<ProgressBar>,
    dry_run: bool,
}

impl CliRunHandler {
    fn new(dry_run: bool) -> Self {
        Self {
            progress: None,
            dry_run,
        }
    }

    fn message_prefix(&self, already_starred: bool) -> &'static str {
        if already_starred {
            "✅ Already starred"
        } else if self.dry_run {
            "⭐ Would star"
        } else {
            "⭐ Starred"
        }
    }

    fn create_progress(total: usize) -> ProgressBar {
        let pb = ProgressBar::with_draw_target(Some(total as u64), ProgressDrawTarget::stdout());
        pb.set_style(
            ProgressStyle::with_template("{spinner:.green} {pos}/{len} ⭐ {wide_msg}")
                .unwrap()
                .tick_chars("⠁⠃⠇⡇⣇⣧⣷⣿"),
        );
        pb.enable_steady_tick(Duration::from_millis(120));
        pb
    }

    fn color_enabled() -> bool {
        supports_color::on_cached(ColorStream::Stdout)
            .map(|level| level.has_basic)
            .unwrap_or(false)
    }
}

impl Drop for CliRunHandler {
    fn drop(&mut self) {
        if let Some(pb) = self.progress.take() {
            pb.finish_and_clear();
        }
    }
}

impl RunEventHandler for CliRunHandler {
    fn on_start(&mut self, total: usize) {
        if total == 0 {
            return;
        }
        let pb = Self::create_progress(total);
        if self.dry_run {
            pb.set_message("Dry run: evaluating repositories...");
        } else {
            pb.set_message("Preparing to star repositories...");
        }
        self.progress = Some(pb);
    }

    fn on_starred(
        &mut self,
        repo: &Repository,
        already_starred: bool,
        _index: usize,
        _total: usize,
    ) {
        let use_color = Self::color_enabled();
        let prefix = self.message_prefix(already_starred);
        let label = if use_color {
            if already_starred {
                format!("{}", prefix.blue().bold())
            } else if self.dry_run {
                format!("{}", prefix.yellow().bold())
            } else {
                format!("{}", prefix.green().bold())
            }
        } else {
            prefix.to_string()
        };
        let repo_url_source = repo.url.clone();
        let repo_url = if use_color {
            format!("{}", repo_url_source.cyan().underline())
        } else {
            repo_url_source
        };

        let via_label_raw = repo.via.as_deref().unwrap_or("unknown source");
        let via_text = if use_color {
            format!(" via {}", via_label_raw.cyan())
        } else {
            format!(" via {via_label_raw}")
        };

        let status_suffix = if already_starred {
            " (already starred)"
        } else {
            ""
        };

        if let Some(pb) = &self.progress {
            pb.set_message(format!("{}{}{}", repo.url, status_suffix, via_text));
            pb.inc(1);
            let line = format!("{label} {repo_url}{status_suffix}{via_text}");
            if pb.is_hidden() {
                println!("{line}");
            } else {
                pb.println(line);
            }
        } else {
            println!("{label} {repo_url}{status_suffix}{via_text}");
        }
    }

    fn on_complete(&mut self, summary: &RunSummary) {
        if let Some(pb) = self.progress.take() {
            pb.finish_and_clear();
        }

        let use_color = Self::color_enabled();

        let already_starred_count = summary
            .starred
            .iter()
            .filter(|repo| repo.already_starred)
            .count();
        let newly_starred_count = summary.starred.len().saturating_sub(already_starred_count);

        if summary.starred.is_empty() {
            let msg = if use_color {
                format!("{}", "🌱 No repositories required starring today.".yellow())
            } else {
                "🌱 No repositories required starring today.".to_string()
            };
            println!("{msg}");
        } else {
            let pluralize = |count: usize| {
                if count == 1 {
                    "repository"
                } else {
                    "repositories"
                }
            };

            if self.dry_run {
                let done = if use_color {
                    format!("{}", "✨ Dry run complete!".yellow().bold())
                } else {
                    "✨ Dry run complete!".to_string()
                };
                let detail = if newly_starred_count > 0 && already_starred_count > 0 {
                    format!(
                        "⭐ {newly_starred_count} {new_plural} would be starred, ✅ {already_starred_count} already starred.",
                        new_plural = pluralize(newly_starred_count)
                    )
                } else if newly_starred_count > 0 {
                    format!(
                        "⭐ {newly_starred_count} {new_plural} would be starred.",
                        new_plural = pluralize(newly_starred_count)
                    )
                } else {
                    format!(
                        "✅ All {already_starred_count} {already_plural} are already starred.",
                        already_plural = pluralize(already_starred_count)
                    )
                };
                let detail = if use_color {
                    format!("{}", detail.clone().white().bold())
                } else {
                    detail
                };
                println!("{done} {detail}");
            } else {
                let done = if use_color {
                    format!("{}", "✨ Completed!".green().bold())
                } else {
                    "✨ Completed!".to_string()
                };
                let detail = if newly_starred_count > 0 && already_starred_count > 0 {
                    format!(
                        "⭐ Starred {newly_starred_count} {new_plural}, ✅ {already_starred_count} already starred.",
                        new_plural = pluralize(newly_starred_count)
                    )
                } else if newly_starred_count > 0 {
                    format!(
                        "⭐ Starred {newly_starred_count} {new_plural}.",
                        new_plural = pluralize(newly_starred_count)
                    )
                } else {
                    format!(
                        "✅ All {already_starred_count} {already_plural} were already starred.",
                        already_plural = pluralize(already_starred_count)
                    )
                };
                let detail = if use_color {
                    format!("{}", detail.clone().white().bold())
                } else {
                    detail
                };
                println!("{done} {detail}");
            }
        }
    }
}

struct MaybeDryRunClient<'a, T: GitHubApi> {
    inner: &'a T,
    dry_run: bool,
}

impl<'a, T: GitHubApi> MaybeDryRunClient<'a, T> {
    fn new(inner: &'a T, dry_run: bool) -> Self {
        Self { inner, dry_run }
    }
}

impl<'a, T: GitHubApi> GitHubApi for MaybeDryRunClient<'a, T> {
    fn viewer_has_starred(&self, owner: &str, repo: &str) -> Result<bool, GitHubError> {
        self.inner.viewer_has_starred(owner, repo)
    }

    fn star(&self, owner: &str, repo: &str) -> Result<(), GitHubError> {
        if self.dry_run {
            Ok(())
        } else {
            self.inner.star(owner, repo)
        }
    }
}

fn load_token(config: &ConfigManager) -> Result<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(token);
        }
    }

    match config.load_token() {
        Ok(token) => Ok(token),
        Err(ConfigError::Io(err)) if err.kind() == io::ErrorKind::NotFound => Err(anyhow!(
            "GitHub token not found. Run `thanks-stars auth --token <token>` or set GITHUB_TOKEN."
        )),
        Err(err) => Err(anyhow!(err)),
    }
}
