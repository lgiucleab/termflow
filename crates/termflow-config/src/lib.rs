//! TOML config: load the global file, then the project file on top, and generate a JSON schema.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "termflow.toml";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub ui: UiConfig,
    pub terminal: TerminalConfig,
    pub updates: UpdatesConfig,
    /// Extra `[[permissions]]` rules, applied after the built-in defaults (last match wins).
    #[cfg_attr(feature = "specta", specta(skip))]
    pub permissions: Vec<termflow_perms::Rule>,
    /// Fallback profiles (`[profiles.<name>]`, `BUILD_PROMPT` §7.6).
    #[cfg_attr(feature = "specta", specta(skip))]
    pub profiles: BTreeMap<String, Profile>,
    /// Scenario routes: which model runs background jobs, compaction, long context, vision.
    #[cfg_attr(feature = "specta", specta(skip))]
    pub routes: Routes,
    #[cfg_attr(feature = "specta", specta(skip))]
    pub catalog: CatalogConfig,
}

/// An ordered fallback chain plus how to move along it. Chain entries are
/// `provider/model@credential-label` (`@…` optional for local providers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Profile {
    pub chain: Vec<String>,
    /// Tried when a request is too large for the current model.
    pub on_context_overflow: Vec<String>,
    /// Tried when the current model refuses (pick another vendor).
    pub on_content_policy: Vec<String>,
    pub retry: RetryConfig,
    /// After a switch, stay on the fallback for at least this many requests.
    pub hysteresis_turns: u32,
    pub switch_back: SwitchBack,
    pub budget: BudgetConfig,
    pub require: Require,
    /// Tell the new model it is continuing another model's session (cross-vendor switches).
    pub handoff_note: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            on_context_overflow: Vec::new(),
            on_content_policy: Vec::new(),
            retry: RetryConfig::default(),
            hysteresis_turns: 3,
            switch_back: SwitchBack::WhenHealthy,
            budget: BudgetConfig::default(),
            require: Require::default(),
            handoff_note: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct RetryConfig {
    /// Wait for a rate limit only if it lifts within this many seconds.
    pub transient_max_wait_s: u64,
    pub overloaded_retries: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            transient_max_wait_s: 10,
            overloaded_retries: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SwitchBack {
    /// Go back to the first model once it is healthy (after the hysteresis turns).
    #[default]
    WhenHealthy,
    /// Stay on the fallback for the rest of the chat.
    Never,
}

/// Spend limits in USD (catalog prices of the model that actually served each request).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct BudgetConfig {
    pub session_usd: Option<f64>,
    pub day_usd: Option<f64>,
    pub month_usd: Option<f64>,
    /// Pause the agent at a limit instead of only warning.
    pub hard_stop: bool,
    /// Warn at this fraction of a limit.
    pub warn_at: f64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            session_usd: None,
            day_usd: None,
            month_usd: None,
            hard_stop: true,
            warn_at: 0.8,
        }
    }
}

/// Capabilities every candidate must have; unknown capabilities don't disqualify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Require {
    pub tools: bool,
    pub vision: bool,
    pub reasoning: bool,
}

impl Default for Require {
    fn default() -> Self {
        Self {
            tools: true,
            vision: false,
            reasoning: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Routes {
    /// Chat titles and other background jobs.
    pub background: Option<String>,
    pub compaction: Option<String>,
    pub long_context: Option<String>,
    pub vision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CatalogConfig {
    /// Fetch models.dev daily (only once a provider is configured).
    pub fetch: Option<bool>,
    /// `"provider/model" = { context = 128000, price = { input = 1.0, output = 2.0 } }`
    pub overrides: BTreeMap<String, termflow_catalog::Override>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(deny_unknown_fields, default)]
pub struct UiConfig {
    /// UI theme id.
    pub theme: String,
    /// UI language (`en`, `es-ES`). Unset = follow the OS.
    pub locale: Option<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "mono-dark".into(),
            locale: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(deny_unknown_fields, default)]
#[allow(clippy::struct_excessive_bools)] // independent user toggles mirrored 1:1 from TOML
pub struct TerminalConfig {
    /// Shell id (`zsh`, `pwsh`, `wsl:Ubuntu`, ...). Unset = the user's login shell.
    pub shell: Option<String>,
    /// Inject `OSC 133` shell integration (needed for blocks).
    pub shell_integration: bool,
    /// Unset = bundled `JetBrains Mono`.
    pub font_family: Option<String>,
    pub font_size: u16,
    pub ligatures: bool,
    pub copy_on_select: bool,
    /// Ask before pasting text that contains line breaks.
    pub warn_multiline_paste: bool,
    /// Lines kept per terminal.
    pub scrollback: u32,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: None,
            shell_integration: true,
            font_family: None,
            font_size: 13,
            ligatures: false,
            copy_on_select: false,
            warn_multiline_paste: true,
            scrollback: 10_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(deny_unknown_fields, default)]
pub struct UpdatesConfig {
    /// Check for updates on startup (the only request made without a configured provider).
    pub check: bool,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self { check: true }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("invalid config: {0}")]
    Invalid(Box<toml::de::Error>),
}

/// Global config directory: `~/.config/termflow` on Unix, the roaming app-data dir on Windows.
pub fn global_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        directories::ProjectDirs::from("com", "flowlabs", "termflow").map(|d| d.config_dir().to_path_buf())
    }
    #[cfg(not(windows))]
    {
        directories::BaseDirs::new().map(|d| d.home_dir().join(".config").join("termflow"))
    }
}

/// Project config file for a workspace.
pub fn project_file(workspace: &Path) -> PathBuf {
    workspace.join(".termflow").join(FILE_NAME)
}

fn read_table(path: &Path) -> Result<Option<toml::Table>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(s) => s.parse::<toml::Table>().map(Some).map_err(|e| ConfigError::Parse {
            path: path.into(),
            source: Box::new(e),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ConfigError::Io {
            path: path.into(),
            source: e,
        }),
    }
}

/// Deep-merge `over` into `base`: tables merge recursively, everything else is replaced.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

/// Load config from the given files (missing files are skipped); later files win.
pub fn load(files: &[&Path]) -> Result<Config, ConfigError> {
    let mut acc = toml::Table::new();
    for f in files {
        if let Some(t) = read_table(f)? {
            merge(&mut acc, t);
        }
    }
    Config::deserialize(toml::Value::Table(acc)).map_err(|e| ConfigError::Invalid(Box::new(e)))
}

/// JSON schema for `termflow.toml`, for editor autocomplete.
pub fn json_schema() -> String {
    let mut s = serde_json::to_string_pretty(&schemars::schema_for!(Config)).unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn missing_files_give_defaults() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load(&[&d.path().join("nope.toml")]).unwrap(), Config::default());
    }

    #[test]
    fn project_overrides_global_deeply() {
        let d = tempfile::tempdir().unwrap();
        let g = write(
            d.path(),
            "g.toml",
            "[ui]\ntheme = \"light\"\nlocale = \"es-ES\"\n[terminal]\nfont_size = 15\n",
        );
        let p = write(d.path(), "p.toml", "[ui]\ntheme = \"high-contrast\"\n");
        let c = load(&[&g, &p]).unwrap();
        assert_eq!(c.ui.theme, "high-contrast");
        assert_eq!(c.ui.locale.as_deref(), Some("es-ES"));
        assert_eq!(c.terminal.font_size, 15);
        assert!(c.updates.check);
    }

    #[test]
    fn profiles_parse_with_defaults() {
        let d = tempfile::tempdir().unwrap();
        let g = write(
            d.path(),
            "g.toml",
            "[profiles.main]\nchain = [\"anthropic/m@work\", \"ollama/local\"]\nbudget = { day_usd = 5.0 }\n[routes]\nbackground = \"ollama/local\"\n[catalog.overrides.\"ollama/local\"]\ncontext = 32000\n",
        );
        let c = load(&[&g]).unwrap();
        let p = &c.profiles["main"];
        assert_eq!(p.chain.len(), 2);
        assert_eq!(
            (p.hysteresis_turns, p.retry.overloaded_retries, p.budget.day_usd),
            (3, 2, Some(5.0))
        );
        assert!(p.budget.hard_stop && p.require.tools && p.handoff_note);
        assert_eq!(c.routes.background.as_deref(), Some("ollama/local"));
        assert_eq!(c.catalog.overrides["ollama/local"].context, Some(32000));
    }

    #[test]
    fn unknown_keys_rejected() {
        let d = tempfile::tempdir().unwrap();
        let g = write(d.path(), "g.toml", "[ui]\nthemee = \"x\"\n");
        assert!(matches!(load(&[&g]), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn checked_in_schema_is_up_to_date() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/termflow.schema.json");
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            on_disk,
            json_schema(),
            "run `cargo run -p termflow-config --bin gen-schema`"
        );
    }
}
