//! Composition root: wires services, the event bus and app state. Knows nothing about Tauri.

pub mod logging;

use std::path::{Path, PathBuf};

use std::sync::Arc;

use termflow_agent::{Chat, ChatOptions};
use termflow_config::Config;
use termflow_router::Router;
use termflow_secrets::{Redactor, SecretStore, Secrets};
use termflow_store::Store;
use termflow_terminal::TerminalManager;
use termflow_types::{AppEvent, AppInfo, Credential, ProviderKind, WireFormat, Workspace};
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
    #[error(transparent)]
    Secret(#[from] termflow_secrets::SecretError),
    #[error("{0}")]
    Invalid(String),
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
    secrets: Secrets,
    chat: Chat,
}

/// What the settings UI sends to add a provider connection.
#[derive(Debug, Clone)]
pub struct NewCredential {
    pub kind: ProviderKind,
    pub label: String,
    /// Overrides the preset's URL; required for `Custom`.
    pub base_url: Option<String>,
    /// Required for `Custom`.
    pub wire: Option<WireFormat>,
    /// Goes straight to the OS keychain. Empty = no key (e.g. local Ollama).
    pub api_key: Option<String>,
}

impl App {
    /// Open the store and load global config. Does not install logging; see [`logging::init`].
    pub fn new(paths: &Paths, redactor: Redactor, secret_store: Arc<dyn SecretStore>) -> Result<Self, CoreError> {
        let config = termflow_config::load(&[&paths.global_config_dir.join(termflow_config::FILE_NAME)])?;
        let store = Store::open(paths.data_dir.join("termflow.db"))?;
        // A turn still streaming when the app died is discarded, never resent (§1.4.3).
        let discarded = store.write_blocking(|c| termflow_store::agent::discard_partial_turns(c))?;
        if discarded > 0 {
            tracing::info!(turns = discarded, "discarded partial turns from a previous run");
        }
        let secrets = Secrets::new(secret_store, redactor.clone());
        let (bus, _) = broadcast::channel(1024);
        let terminals = TerminalManager::new(
            store.clone(),
            bus.clone(),
            paths.data_dir.join("shell-integration"),
            config.terminal.shell_integration,
        )?;
        tracing::info!(data_dir = %paths.data_dir.display(), "core started");
        let router = Arc::new(Router::new(store.clone(), secrets.clone()));
        let mut rules = termflow_perms::default_rules();
        rules.extend(config.permissions.iter().cloned());
        let blocks_store = store.clone();
        let blocks: termflow_tools::BlockLookup = Arc::new(move |id| {
            let r = blocks_store.reader().ok()?;
            let id = match id {
                Some(id) => id,
                None => termflow_store::blocks::recent(&r, 1).ok()?.into_iter().next()?.id,
            };
            termflow_store::blocks::get(&r, id).ok()?
        });
        let chat = Chat::new(
            store.clone(),
            router,
            bus.clone(),
            ChatOptions {
                rules,
                global_dir: Some(paths.global_config_dir.clone()),
                blocks: Some(blocks),
                shell: None,
            },
        );
        Ok(Self {
            config,
            store,
            bus,
            redactor,
            terminals,
            secrets,
            chat,
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

    pub fn chat(&self) -> &Chat {
        &self.chat
    }

    pub fn credentials(&self) -> Result<Vec<Credential>, CoreError> {
        Ok(termflow_store::agent::list_credentials(&self.store.reader()?)?)
    }

    /// Store the key in the keychain, then the metadata in SQLite. The key never touches disk.
    pub async fn add_credential(&self, n: NewCredential) -> Result<Credential, CoreError> {
        let (base_url, wire) = match (n.kind.defaults(), n.base_url.filter(|u| !u.trim().is_empty()), n.wire) {
            (_, Some(url), Some(wire)) => (url, wire),
            (Some((url, wire)), custom_url, _) => (custom_url.unwrap_or_else(|| url.to_owned()), wire),
            (None, _, _) => {
                return Err(CoreError::Invalid(
                    "a custom endpoint needs a base URL and a wire format".into(),
                ));
            }
        };
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err(CoreError::Invalid(
                "base URL must start with http:// or https://".into(),
            ));
        }
        let key = n.api_key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
        let cred = Credential {
            id: ulid::Ulid::new().to_string(),
            kind: n.kind,
            label: if n.label.trim().is_empty() {
                n.kind.id().to_owned()
            } else {
                n.label.trim().to_owned()
            },
            base_url: base_url.trim_end_matches('/').to_owned(),
            wire,
            has_secret: key.is_some(),
            created_at: jiff::Timestamp::now(),
        };
        if let Some(k) = &key {
            self.secrets.set(&cred.id, k)?;
        }
        let c = cred.clone();
        if let Err(e) = self
            .store
            .write(move |conn| termflow_store::agent::insert_credential(conn, &c))
            .await
        {
            let _ = self.secrets.delete(&cred.id); // don't leave an orphaned key behind
            return Err(e.into());
        }
        Ok(cred)
    }

    pub fn recent_workspaces(&self) -> Result<Vec<Workspace>, CoreError> {
        Ok(termflow_store::agent::recent_workspaces(&self.store.reader()?, 20)?)
    }

    /// Open (or re-open) a project folder; it must exist.
    pub async fn open_workspace(&self, path: String) -> Result<Workspace, CoreError> {
        let canon = std::path::Path::new(&path)
            .canonicalize()
            .map_err(|e| CoreError::Invalid(format!("cannot open {path}: {e}")))?;
        if !canon.is_dir() {
            return Err(CoreError::Invalid(format!("{path} is not a folder")));
        }
        let p = canon.display().to_string();
        Ok(self
            .store
            .write(move |c| termflow_store::agent::open_workspace(c, &p))
            .await?)
    }

    pub async fn forget_workspace(&self, id: String) -> Result<(), CoreError> {
        Ok(self
            .store
            .write(move |c| termflow_store::agent::forget_workspace(c, &id))
            .await?)
    }

    pub fn workspace(&self, id: &str) -> Result<Option<Workspace>, CoreError> {
        Ok(termflow_store::agent::get_workspace(&self.store.reader()?, id)?)
    }

    /// Files for the `@` picker.
    pub fn project_files(&self, workspace_id: &str) -> Result<Vec<String>, CoreError> {
        let w = self
            .workspace(workspace_id)?
            .ok_or_else(|| CoreError::Invalid("unknown project".into()))?;
        Ok(termflow_context::project_files(std::path::Path::new(&w.path), 5000))
    }

    pub async fn delete_credential(&self, id: String) -> Result<(), CoreError> {
        self.secrets.delete(&id)?;
        self.store
            .write(move |c| termflow_store::agent::delete_credential(c, &id))
            .await?;
        Ok(())
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
        let app = App::new(
            &Paths::under(dir.path()),
            Redactor::new(),
            Arc::new(termflow_secrets::MemoryStore::default()),
        )
        .unwrap();
        assert_eq!(app.app_info().name, "TermFlow");
        assert!(dir.path().join("data/shell-integration/zsh/.zshenv").is_file());
        app.set_setting("layout".into(), "{}".into()).await.unwrap();
        assert_eq!(app.setting("layout").unwrap().as_deref(), Some("{}"));
    }

    #[tokio::test]
    async fn api_key_lives_only_in_the_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let mem = termflow_secrets::MemoryStore::default();
        let app = App::new(&Paths::under(dir.path()), Redactor::new(), Arc::new(mem.clone())).unwrap();
        let key = "sk-ant-api03-SECRET-should-never-hit-disk";
        let cred = app
            .add_credential(NewCredential {
                kind: ProviderKind::Anthropic,
                label: "personal".into(),
                base_url: None,
                wire: None,
                api_key: Some(key.into()),
            })
            .await
            .unwrap();
        assert!(cred.has_secret);
        assert_eq!(cred.base_url, "https://api.anthropic.com");
        assert_eq!(
            termflow_secrets::SecretStore::get(&mem, &cred.id).unwrap().as_deref(),
            Some(key)
        );
        // Nothing under the data dir (db, wal, shm, logs) contains the key.
        for entry in walk(dir.path()) {
            let bytes = std::fs::read(&entry).unwrap_or_default();
            assert!(
                !bytes.windows(key.len()).any(|w| w == key.as_bytes()),
                "key found in {}",
                entry.display()
            );
        }
        app.delete_credential(cred.id.clone()).await.unwrap();
        assert_eq!(termflow_secrets::SecretStore::get(&mem, &cred.id).unwrap(), None);
        assert_eq!(app.credentials().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn custom_endpoint_needs_url_and_wire() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &Paths::under(dir.path()),
            Redactor::new(),
            Arc::new(termflow_secrets::MemoryStore::default()),
        )
        .unwrap();
        let bad = NewCredential {
            kind: ProviderKind::Custom,
            label: String::new(),
            base_url: None,
            wire: None,
            api_key: None,
        };
        assert!(matches!(app.add_credential(bad).await, Err(CoreError::Invalid(_))));
        let ok = NewCredential {
            kind: ProviderKind::Custom,
            label: "lm studio".into(),
            base_url: Some("http://localhost:1234/v1/".into()),
            wire: Some(WireFormat::OpenaiChat),
            api_key: None,
        };
        let c = app.add_credential(ok).await.unwrap();
        assert_eq!((c.base_url.as_str(), c.has_secret), ("http://localhost:1234/v1", false));
    }

    fn walk(p: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
            let path = e.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }
}
