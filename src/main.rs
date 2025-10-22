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
use thanks_stars::github::{GitHubClient, GitHubError};
use thanks_stars::{run_with_handler, RunError, RunEventHandler, RunSummary};

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Star the GitHub repositories of your dependencies."
)]
struct Cli {
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

#[derive(Args, Default)]
struct RunArgs {
    /// Path to the project root. Defaults to the current directory.
    #[arg(short, long)]
    path: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = ConfigManager::new()?;

    match cli.command.unwrap_or(Commands::Run(RunArgs::default())) {
        Commands::Auth(args) => handle_auth(args, &config),
        Commands::Run(args) => handle_run(args, &config),
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

    let mut handler = CliRunHandler::default();
    run_with_handler(&root, &client, &mut handler).map_err(|err| match err {
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

#[derive(Default)]
struct CliRunHandler {
    progress: Option<ProgressBar>,
}

impl CliRunHandler {
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
        pb.set_message("Preparing to star repositories...");
        self.progress = Some(pb);
    }

    fn on_starred(&mut self, repo: &Repository, _index: usize, _total: usize) {
        let use_color = Self::color_enabled();
        let label = if use_color {
            format!("{}", "⭐ Starred".green().bold())
        } else {
            "⭐ Starred".to_string()
        };
        let repo_url_source = repo.url.clone();
        let repo_url = if use_color {
            format!("{}", repo_url_source.cyan().underline())
        } else {
            repo_url_source
        };

        if let Some(pb) = &self.progress {
            pb.set_message(repo.url.clone());
            pb.inc(1);
            let line = format!("{label} {repo_url}");
            if pb.is_hidden() {
                println!("{line}");
            } else {
                pb.println(line);
            }
        } else {
            println!("{label} {repo_url}");
        }
    }

    fn on_complete(&mut self, summary: &RunSummary) {
        if let Some(pb) = self.progress.take() {
            pb.finish_and_clear();
        }

        let use_color = Self::color_enabled();

        if summary.starred.is_empty() {
            let msg = if use_color {
                format!("{}", "🌱 No repositories required starring today.".yellow())
            } else {
                "🌱 No repositories required starring today.".to_string()
            };
            println!("{msg}");
        } else {
            let done = if use_color {
                format!("{}", "✨ Completed!".green().bold())
            } else {
                "✨ Completed!".to_string()
            };
            let total_message = format!("Starred {} repositories.", summary.starred.len());
            let total = if use_color {
                format!("{}", total_message.clone().white().bold())
            } else {
                total_message
            };
            println!("{done} {total}");
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
