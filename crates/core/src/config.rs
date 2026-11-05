use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The default config, kept in sync with the file at the repo root.
pub const DEFAULT_CONFIG: &str = include_str!("../../../config.example.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub provider: Provider,
    pub linux: Linux,
    pub system_prompt: String,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub default_action: String,
    pub auto_paste: bool,
    pub hotkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Provider {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub api_key_env: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Linux {
    pub grab: GrabSource,
    pub grab_cmds: Vec<Vec<String>>,
    pub paste_cmds: Vec<Vec<String>>,
    pub fallback_to_clipboard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrabSource {
    Primary,
    Clipboard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub key: String,
    pub prompt: String,
}

impl Action {
    pub fn render(&self, text: &str) -> String {
        if self.prompt.contains("{text}") {
            self.prompt.replace("{text}", text)
        } else {
            format!("{}\n\n{}", self.prompt.trim_end(), text)
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(DEFAULT_CONFIG).expect("config.example.toml must parse")
    }
}
impl Default for General {
    fn default() -> Self {
        Config::default().general
    }
}
impl Default for Provider {
    fn default() -> Self {
        Config::default().provider
    }
}
impl Default for Linux {
    fn default() -> Self {
        Config::default().linux
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/mirzabenevis/config.toml`
    pub fn path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "mirzabenevis").context("no home directory")?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load the user config, falling back to the embedded default if the file is missing.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    /// Write the default config to disk (does not overwrite an existing one).
    pub fn init() -> Result<PathBuf> {
        let path = Self::path()?;
        if path.exists() {
            anyhow::bail!("{} already exists", path.display());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, DEFAULT_CONFIG)?;
        Ok(path)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(path)
    }

    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    pub fn action_by_key(&self, key: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.key == key)
    }
}

impl Provider {
    /// Resolve the API key: explicit value first, then the named env var.
    pub fn api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.trim().to_string());
        }
        if !self.api_key_env.is_empty() {
            if let Ok(v) = std::env::var(&self.api_key_env) {
                if !v.trim().is_empty() {
                    return Some(v.trim().to_string());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_and_has_actions() {
        let c = Config::default();
        assert!(!c.actions.is_empty());
        assert!(c.action(&c.general.default_action).is_some());
    }

    #[test]
    fn render_substitutes_text() {
        let a = Action {
            id: "x".into(),
            name: "x".into(),
            key: "".into(),
            prompt: "Fix: {text}".into(),
        };
        assert_eq!(a.render("hi"), "Fix: hi");
    }
}
