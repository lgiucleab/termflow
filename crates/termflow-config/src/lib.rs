//! TOML config: load the global file, then the project file on top, and generate a JSON schema.

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
            theme: "dark".into(),
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
