//! Composition root: wires services, the event bus and app state. Knows nothing about Tauri.

pub mod logging;

use std::path::{Path, PathBuf};

use termflow_config::Config;
use termflow_secrets::Redactor;
use termflow_store::Store;
use termflow_terminal::TerminalManager;
use termflow_types::{AppEvent, AppInfo};
use tokio::sync::broadcast;

pub use termflow_types as types;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no home directory found")]
    NoHome,
    #[error(transparent)]
    Config(#[from] termflow_config::ConfigError),
    #[error(transparent)]
    Store(#[from] termflow_store::StoreError),
    #[error(transparent)]
    Terminal(#[from] termflow_terminal::TerminalError),
}

/// Where the app keeps its files.
#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub global_config_dir: PathBuf,
}

impl Paths {
    /// OS-appropriate locations.
    pub fn os_default() -> Result<Self, CoreError> {
        let dirs = directories::ProjectDirs::from("com", "flowlabs", "termflow").ok_or(CoreError::NoHome)?;
        Ok(Self {
            data_dir: dirs.data_dir().to_path_buf(),
            log_dir: dirs.data_local_dir().join("logs"),
            global_config_dir: termflow_config::global_dir().ok_or(CoreError::NoHome)?,
        })
    }

    /// Everything under one directory (tests, portable mode).
    pub fn under(root: &Path) -> Self {
        Self {
            data_dir: root.join("data"),
            log_dir: root.join("logs"),
            global_config_dir: root.join("config"),
        }
    }
}

pub struct App {
    config: Config,
    store: Store,
    bus: broadcast::Sender<AppEvent>,
    redactor: Redactor,
    terminals: TerminalManager,
}

impl App {
    /// Open the store and load global config. Does not install logging; see [`logging::init`].
    pub fn new(paths: &Paths, redactor: Redactor) -> Result<Self, CoreError> {
        let config = termflow_config::load(&[&paths.global_config_dir.join(termflow_config::FILE_NAME)])?;
        let store = Store::open(paths.data_dir.join("termflow.db"))?;
        let (bus, _) = broadcast::channel(1024);
        let terminals = TerminalManager::new(
            store.clone(),
            bus.clone(),
            paths.data_dir.join("shell-integration"),
            config.terminal.shell_integration,
        )?;
        tracing::info!(data_dir = %paths.data_dir.display(), "core started");
        Ok(Self {
            config,
            store,
            bus,
            redactor,
            terminals,
        })
    }

    pub fn app_info(&self) -> AppInfo {
        AppInfo {
            name: "TermFlow".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.bus.subscribe()
    }

    pub fn terminals(&self) -> &TerminalManager {
        &self.terminals
    }

    /// Small UI state blobs (e.g. tab layout) in the `settings` table.
    pub fn setting(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(termflow_store::settings::get(&self.store.reader()?, key)?)
    }

    pub async fn set_setting(&self, key: String, value_json: String) -> Result<(), CoreError> {
        Ok(self
            .store
            .write(move |c| termflow_store::settings::set(c, &key, &value_json))
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn starts_and_persists_settings() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(&Paths::under(dir.path()), Redactor::new()).unwrap();
        assert_eq!(app.app_info().name, "TermFlow");
        assert!(dir.path().join("data/shell-integration/zsh/.zshenv").is_file());
        app.set_setting("layout".into(), "{}".into()).await.unwrap();
        assert_eq!(app.setting("layout").unwrap().as_deref(), Some("{}"));
    }
}
