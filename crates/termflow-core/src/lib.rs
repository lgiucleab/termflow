//! Composition root: wires services, the event bus and app state. Knows nothing about Tauri.

pub mod logging;

use std::path::{Path, PathBuf};
use std::time::Duration;

use termflow_config::Config;
use termflow_secrets::Redactor;
use termflow_store::Store;
use termflow_types::{AppEvent, AppInfo};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

pub use termflow_types as types;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no home directory found")]
    NoHome,
    #[error(transparent)]
    Config(#[from] termflow_config::ConfigError),
    #[error(transparent)]
    Store(#[from] termflow_store::StoreError),
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
}

impl App {
    /// Open the store and load global config. Does not install logging; see [`logging::init`].
    pub fn new(paths: &Paths, redactor: Redactor) -> Result<Self, CoreError> {
        let config = termflow_config::load(&[&paths.global_config_dir.join(termflow_config::FILE_NAME)])?;
        let store = Store::open(paths.data_dir.join("termflow.db"))?;
        let (bus, _) = broadcast::channel(1024);
        tracing::info!(data_dir = %paths.data_dir.display(), "core started");
        Ok(Self {
            config,
            store,
            bus,
            redactor,
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

    /// M0 demo: publish `Tick` every `every` until cancelled. Must be called inside a tokio runtime.
    pub fn start_demo_ticker(&self, every: Duration, cancel: CancellationToken) {
        let bus = self.bus.clone();
        tokio::spawn(async move {
            let mut n = 0u32;
            let mut interval = tokio::time::interval(every);
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    _ = interval.tick() => {
                        n = n.wrapping_add(1);
                        let _ = bus.send(AppEvent::Tick { n }); // no subscribers is fine
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ticker_publishes_until_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(&Paths::under(dir.path()), Redactor::new()).unwrap();
        assert_eq!(app.app_info().name, "TermFlow");
        let mut rx = app.subscribe();
        let cancel = CancellationToken::new();
        app.start_demo_ticker(Duration::from_millis(5), cancel.clone());
        assert_eq!(rx.recv().await.unwrap(), AppEvent::Tick { n: 1 });
        assert_eq!(rx.recv().await.unwrap(), AppEvent::Tick { n: 2 });
        cancel.cancel();
    }
}
