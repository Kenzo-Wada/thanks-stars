use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

const CONFIG_ENV: &str = "THANKS_STARS_CONFIG_DIR";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("unable to determine configuration directory")]
    MissingDirectory,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("{0}")]
    TomlDe(#[from] toml::de::Error),
}

#[derive(Debug, Serialize, Deserialize)]
struct RawConfig {
    token: String,
}

#[derive(Debug, Clone)]
pub struct ConfigManager {
    base_dir: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Result<Self, ConfigError> {
        let dir = determine_base_dir()?;
        Ok(Self { base_dir: dir })
    }

    pub fn with_base_dir<P: Into<PathBuf>>(base_dir: P) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn save_token(&self, token: &str) -> Result<(), ConfigError> {
        fs::create_dir_all(&self.base_dir)?;
        let config = RawConfig {
            token: token.to_string(),
        };
        let contents = toml::to_string(&config)?;
        fs::write(self.config_file(), contents)?;
        Ok(())
    }

    pub fn load_token(&self) -> Result<String, ConfigError> {
        let contents = fs::read_to_string(self.config_file())?;
        let config: RawConfig = toml::from_str(&contents)?;
        Ok(config.token)
    }

    pub fn config_file(&self) -> PathBuf {
        self.base_dir.join(CONFIG_FILE)
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

fn determine_base_dir() -> Result<PathBuf, ConfigError> {
    if let Ok(path) = env::var(CONFIG_ENV) {
        return Ok(PathBuf::from(path));
    }

    let dirs = ProjectDirs::from("dev", "thanks-stars", "thanks-stars")
        .ok_or(ConfigError::MissingDirectory)?;
    Ok(dirs.config_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn saves_and_loads_token() {
        let dir = tempdir().unwrap();
        let manager = ConfigManager::with_base_dir(dir.path());

        manager.save_token("abc123").unwrap();
        let loaded = manager.load_token().unwrap();

        assert_eq!(loaded, "abc123");
        assert!(manager.config_file().exists());
    }

    #[test]
    fn load_missing_token_returns_error() {
        let dir = tempdir().unwrap();
        let manager = ConfigManager::with_base_dir(dir.path());

        let err = manager.load_token().unwrap_err();

        match err {
            ConfigError::Io(io_err) => assert_eq!(io_err.kind(), std::io::ErrorKind::NotFound),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
