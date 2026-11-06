use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The default config, kept in sync with the file at the repo root.
pub const DEFAULT_CONFIG: &str = include_str!("../../../config.example.toml");

// Missing keys fall back to the embedded default config. `Config` has no
// container-level `#[serde(default)]` on purpose: that would call
// `Config::default()`, which parses the TOML, which calls it again (stack overflow).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub provider: Provider,
    #[serde(default)]
    pub linux: Linux,
    #[serde(default)]
    pub translate: Translate,
    #[serde(default = "default_actions")]
    pub actions: Vec<Action>,
    #[serde(default = "default_modifiers")]
    pub modifiers: Vec<Modifier>,
    /// Extra instruction lines for the next run (modifiers, context), added to
    /// the action's prompt before the text. Set per run by the popup; not saved.
    #[serde(skip)]
    pub run_extra: String,
}

/// `{target}` in a prompt is replaced by `target`; the popup's T key cycles
/// through `targets` and saves the choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Translate {
    pub target: String,
    pub targets: Vec<String>,
}

/// A toggle in the popup that appends `suffix` to the next run's prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Modifier {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub key: String,
    pub suffix: String,
}

/// What happens to an action's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    /// Paste it over the selection.
    #[default]
    Replace,
    /// Put it on the clipboard.
    Clipboard,
    /// Only show it (CLI: print it).
    Show,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub default_action: String,
    pub auto_paste: bool,
    pub hotkey: String,
    /// CLI only: show a desktop notification (`notify-send`) on errors and fallbacks
    /// when not run from a terminal.
    pub notify: bool,
    /// Longer input is refused before anything is sent.
    pub max_input_chars: usize,
    /// Keep successful runs in the history file (undo, recent instructions).
    pub history: bool,
    /// Language for `{explain_in}` (Explain, the popup's explain-change);
    /// empty = the language of the text.
    pub explain_in: String,
}

/// Which AI backend to use. See [`crate::backends`] for what gets detected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Provider {
    /// `"auto"` (first backend found, following `order`) or a backend id.
    #[serde(rename = "use")]
    pub use_backend: String,
    /// Preference order for `use = "auto"`. Unknown ids are ignored; backends
    /// missing from the list are tried last, in catalog order.
    pub order: Vec<String>,
    /// Per-backend model overrides, e.g. `deepseek = "deepseek-v4-pro"`.
    pub models: BTreeMap<String, String>,
    pub max_tokens: u32,
    /// Sent only to backends that accept it (reasoning models reject it).
    pub temperature: f64,
    pub timeout_secs: u64,
    /// Your own backend; detected when `base_url` or `command` is set.
    pub custom: Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Custom {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub api_key_env: String,
    /// For `kind = "command"`: argv of a program that reads the prompt on stdin
    /// and prints the answer. `{system}` and `{model}` are substituted.
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Command,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Linux {
    pub grab: GrabSource,
    pub grab_cmds: Vec<Vec<String>>,
    /// With `grab = "primary"`: use the clipboard when nothing is selected.
    pub grab_fallback_clipboard: bool,
    /// How to put the result back: `clipboard` = set clipboard + simulate Ctrl+V
    /// with `paste_key_cmds` (restores the clipboard after); `type` = pipe the
    /// text into `paste_cmds`. `clipboard` falls back to `type`, then either
    /// leaves the text on the clipboard (`fallback_to_clipboard`).
    pub paste: PasteMode,
    pub paste_key_cmds: Vec<Vec<String>>,
    pub paste_cmds: Vec<Vec<String>>,
    pub fallback_to_clipboard: bool,
    /// Ask your login shell for its environment when an API key or a CLI is not
    /// visible to this process (apps started by the desktop often miss what
    /// `~/.bashrc` / `~/.zshrc` export).
    pub login_shell_env: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrabSource {
    Primary,
    Clipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PasteMode {
    #[default]
    Clipboard,
    Type,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Action {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub key: String,
    pub prompt: String,
    /// Backend id for this action, overriding `provider.use` (and the popup/CLI choice).
    #[serde(default)]
    pub backend: Option<String>,
    /// Model for this action, overriding the backend's model.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub output: Output,
}

impl Action {
    /// The user message: the prompt with the text inside `<text>` tags, so the
    /// model can tell the data apart from the instruction. A `</text>` inside
    /// the text is escaped so it cannot close the wrapper.
    pub fn render(&self, text: &str) -> String {
        self.render_with(text, "")
    }

    /// [`Action::render`] with `extra` instructions just before the text. Only
    /// the prompt's first `{text}` is filled; nothing inserted is expanded again.
    pub fn render_with(&self, text: &str, extra: &str) -> String {
        let wrapped = format!("<text>\n{}\n</text>", text.replace("</text>", "<\\/text>"));
        let block = if extra.is_empty() {
            wrapped
        } else {
            format!("{extra}\n\n{wrapped}")
        };
        if self.prompt.contains("{text}") {
            self.prompt.replacen("{text}", &block, 1)
        } else {
            format!("{}\n\n{}", self.prompt.trim_end(), block)
        }
    }
}

/// Keys the popup uses itself; an action or modifier bound to one is ignored.
pub const RESERVED_KEYS: &[&str] = &[
    "enter", "escape", "esc", "r", "c", "d", "p", "e", "/", "u", "t", "k", "x", "m", "n",
];

/// Lowercase `key`; drop it (with a warning) when the popup reserves it.
fn fix_key(kind: &str, id: &str, key: &mut String) {
    *key = key.trim().to_lowercase();
    if RESERVED_KEYS.contains(&key.as_str()) {
        tracing::warn!("{kind} `{id}`: key \"{key}\" is reserved by the popup, ignored");
        key.clear();
    }
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(DEFAULT_CONFIG).expect("config.example.toml must parse")
    }
}

/// The parts of the embedded config that are not plain scalars.
#[derive(Deserialize)]
struct Embedded {
    system_prompt: String,
    actions: Vec<Action>,
    modifiers: Vec<Modifier>,
}

fn embedded() -> Embedded {
    toml::from_str(DEFAULT_CONFIG).expect("config.example.toml must parse")
}
fn default_system_prompt() -> String {
    embedded().system_prompt
}
pub fn default_actions() -> Vec<Action> {
    embedded().actions
}
pub fn default_modifiers() -> Vec<Modifier> {
    embedded().modifiers
}

/// Default auto-detection order: direct APIs first (fastest), then coding-agent
/// CLIs you are logged in to, then local model servers.
pub const DEFAULT_ORDER: &[&str] = &[
    "custom",
    "anthropic",
    "deepseek",
    "openai",
    "gemini",
    "openrouter",
    "groq",
    "mistral",
    "xai",
    "qwen",
    "together",
    "cerebras",
    "moonshot",
    "claude-code",
    "codex",
    "llm",
    "ollama",
    "lmstudio",
    "llamacpp",
    "vllm",
    "jan",
];

// These literals mirror config.example.toml; the `example_matches_defaults`
// test fails if the two drift apart.
impl Default for General {
    fn default() -> Self {
        Self {
            default_action: "fix".into(),
            auto_paste: true,
            hotkey: String::new(),
            notify: true,
            max_input_chars: 40_000,
            history: true,
            explain_in: String::new(),
        }
    }
}
impl Default for Provider {
    fn default() -> Self {
        Self {
            use_backend: "auto".into(),
            order: DEFAULT_ORDER.iter().map(|s| s.to_string()).collect(),
            models: BTreeMap::new(),
            max_tokens: 16384,
            temperature: 0.2,
            timeout_secs: 90,
            custom: Custom::default(),
        }
    }
}
impl Default for Translate {
    fn default() -> Self {
        Self {
            target: "en".into(),
            targets: vec!["en".into(), "de".into(), "fa".into()],
        }
    }
}
impl Default for Custom {
    fn default() -> Self {
        Self {
            kind: ProviderKind::OpenAi,
            base_url: String::new(),
            model: String::new(),
            api_key: String::new(),
            api_key_env: String::new(),
            command: Vec::new(),
        }
    }
}
impl Default for Linux {
    fn default() -> Self {
        let cmd = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        Self {
            grab: GrabSource::Primary,
            grab_cmds: vec![
                cmd(&["wl-paste", "--primary", "--no-newline"]),
                cmd(&["xclip", "-o", "-selection", "primary"]),
            ],
            grab_fallback_clipboard: false,
            paste: PasteMode::Clipboard,
            paste_key_cmds: vec![
                cmd(&["wtype", "-M", "ctrl", "v", "-m", "ctrl"]),
                cmd(&["ydotool", "key", "29:1", "47:1", "47:0", "29:0"]),
                cmd(&["xdotool", "key", "--clearmodifiers", "ctrl+v"]),
            ],
            paste_cmds: vec![
                cmd(&["wtype", "-"]),
                cmd(&["ydotool", "type", "--file", "/dev/stdin"]),
                cmd(&["xdotool", "type", "--clearmodifiers", "--file", "-"]),
            ],
            fallback_to_clipboard: true,
            login_shell_env: true,
        }
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
        Self::from_user_toml(&raw).with_context(|| format!("in {}", path.display()))
    }

    /// Parse a user config file and check its keys ([`Config::check_keys`]).
    pub fn from_user_toml(raw: &str) -> Result<Self> {
        let mut cfg: Config = toml::from_str(raw)?;
        let own_modifiers = raw.parse::<toml::Table>()?.contains_key("modifiers");
        cfg.check_keys(own_modifiers)?;
        Ok(cfg)
    }

    /// Keys are lowercased. Reserved keys are dropped with a warning, and so
    /// is a built-in modifier's key when one of your actions uses it. Two
    /// actions with the same `id` or `key`, or an action and one of your own
    /// modifiers with the same key, are an error.
    pub fn check_keys(&mut self, own_modifiers: bool) -> Result<()> {
        for a in &mut self.actions {
            fix_key("action", &a.id, &mut a.key);
        }
        for m in &mut self.modifiers {
            fix_key("modifier", &m.id, &mut m.key);
            if !own_modifiers && !m.key.is_empty() && self.actions.iter().any(|a| a.key == m.key) {
                tracing::warn!(
                    "modifier `{}`: key \"{}\" is taken by an action, ignored",
                    m.id,
                    m.key
                );
                m.key.clear();
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut dups = Vec::new();
        let all = (self.actions.iter().map(|a| ("id", &a.id)))
            .chain(self.actions.iter().map(|a| ("key", &a.key)))
            .chain(self.modifiers.iter().map(|m| ("modifier id", &m.id)))
            .chain(self.modifiers.iter().map(|m| ("key", &m.key)));
        for (what, v) in all {
            if !v.is_empty() && !seen.insert((what, v)) {
                dups.push(format!("{what} \"{v}\""));
            }
        }
        if !dups.is_empty() {
            anyhow::bail!("duplicate action {}", dups.join(", "));
        }
        Ok(())
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

    /// Edit the config file with `edit`, keeping its comments and layout, and
    /// write it only if the result still loads ([`Config::from_user_toml`]).
    /// Starts from an empty file if there is none, so the user keeps receiving
    /// improved defaults (actions, prompt, order) from future versions.
    /// Returns the config as now saved.
    pub fn edit_file(edit: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<()>) -> Result<Self> {
        let file = Self::path()?;
        let raw = if file.exists() {
            std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?
        } else {
            String::new()
        };
        let mut doc: toml_edit::DocumentMut = raw.parse().context("config is not valid TOML")?;
        edit(&mut doc)?;
        let updated = doc.to_string();
        let cfg = Self::from_user_toml(&updated).context("edited config would not load")?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&file, updated)?;
        Ok(cfg)
    }

    /// Set one value in the config file (see [`Config::edit_file`]).
    /// `path` is a dotted key path such as `["provider", "use"]`.
    pub fn set_in_file(path: &[&str], value: impl Into<toml_edit::Value>) -> Result<PathBuf> {
        Self::edit_file(|doc| set_doc_value(doc, path, value))?;
        Self::path()
    }

    /// Save `prompt` as a new click-only action ([`add_action_to`]).
    pub fn add_action(prompt: &str) -> Result<Self> {
        Self::edit_file(|doc| add_action_to(doc, prompt).map(drop))
    }

    /// Remove an action added with [`Config::add_action`] ([`remove_action_from`]).
    pub fn remove_action(&self, id: &str) -> Result<Self> {
        Self::edit_file(|doc| remove_action_from(doc, id, &self.general.default_action))
    }

    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    pub fn action_by_key(&self, key: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.key == key)
    }

    /// The instructions the chosen modifiers and a context line add to a run.
    pub fn extras(&self, modifier_ids: &[String], context: &str) -> String {
        let mut lines: Vec<String> = self
            .modifiers
            .iter()
            .filter(|m| modifier_ids.contains(&m.id))
            .map(|m| m.suffix.trim().to_string())
            .collect();
        let context = context.trim();
        if !context.is_empty() {
            lines.push(format!("Context: {context}"));
        }
        lines.join("\n")
    }
}

/// Pure helper behind [`Config::set_in_file`].
pub fn set_toml_value(
    raw: &str,
    path: &[&str],
    value: impl Into<toml_edit::Value>,
) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = raw.parse().context("config is not valid TOML")?;
    set_doc_value(&mut doc, path, value)?;
    Ok(doc.to_string())
}

/// Set `path` (creating its tables) to `value`, keeping the comment after an
/// existing value.
pub fn set_doc_value(
    doc: &mut toml_edit::DocumentMut,
    path: &[&str],
    value: impl Into<toml_edit::Value>,
) -> Result<()> {
    let (last, parents) = path.split_last().context("empty key path")?;
    let mut table = doc.as_table_mut();
    for key in parents {
        let entry = table
            .entry(key)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        table = entry
            .as_table_mut()
            .with_context(|| format!("`{key}` is not a table"))?;
    }
    put(table, last, value.into());
    Ok(())
}

fn put(table: &mut toml_edit::Table, key: &str, mut value: toml_edit::Value) {
    match table.get_mut(key).and_then(|i| i.as_value_mut()) {
        Some(v) => {
            *value.decor_mut() = v.decor().clone();
            *v = value;
        }
        None => {
            value.decor_mut().clear();
            table.insert(key, toml_edit::Item::Value(value));
        }
    }
}

/// A value from the popup's settings as TOML: text, a boolean, a number
/// (integers stay integers) or a list of texts.
pub fn toml_value(v: &serde_json::Value) -> Result<toml_edit::Value> {
    use serde_json::Value as J;
    Ok(match v {
        J::String(s) => s.as_str().into(),
        J::Bool(b) => (*b).into(),
        J::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().context("not a number")?.into(),
        },
        J::Array(a) => a
            .iter()
            .map(|x| x.as_str().context("a list can only hold text"))
            .collect::<Result<toml_edit::Array>>()?
            .into(),
        _ => anyhow::bail!("cannot save {v} in the config"),
    })
}

/// The file's `[[actions]]` or `[[modifiers]]` (`list`). Yours replace the
/// built-in ones, so a file without any first gets a copy of them
/// (`materialise`).
fn list_of<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    list: &str,
    materialise: bool,
) -> Result<&'a mut toml_edit::ArrayOfTables> {
    if materialise && !doc.contains_key(list) {
        // A built-in modifier loses a key one of your actions uses, as when loading.
        let loaded = Config::from_user_toml(&doc.to_string())?;
        let defaults: toml_edit::DocumentMut = DEFAULT_CONFIG.parse()?;
        let mut aot = defaults[list]
            .as_array_of_tables()
            .with_context(|| format!("no default {list}"))?
            .clone();
        // After everything already in the file (modifiers after actions).
        let base = if list == "actions" {
            1_000_000
        } else {
            2_000_000
        };
        for (i, t) in aot.iter_mut().enumerate() {
            t.set_position(base + i);
            if list == "modifiers" && loaded.modifiers.get(i).is_some_and(|m| m.key.is_empty()) {
                t.remove("key");
            }
        }
        doc.insert(list, toml_edit::Item::ArrayOfTables(aot));
    }
    doc.get_mut(list)
        .and_then(|i| i.as_array_of_tables_mut())
        .with_context(|| format!("`{list}` is not a list of [[{list}]] tables"))
}

/// "action" or "modifier"
fn noun(list: &str) -> &str {
    list.strip_suffix('s').unwrap_or(list)
}

fn id_of(t: &toml_edit::Table) -> Option<&str> {
    t.get("id").and_then(|v| v.as_str())
}

/// Index of the entry `id` in the file's `list`.
fn index_of(aot: &toml_edit::ArrayOfTables, list: &str, id: &str) -> Result<usize> {
    aot.iter()
        .position(|t| id_of(t) == Some(id))
        .with_context(|| format!("no {} `{id}` in the config file", noun(list)))
}

/// The built-in entries of `list`, as written in the default config.
fn default_entries(list: &str) -> Result<Vec<toml_edit::Table>> {
    let defaults: toml_edit::DocumentMut = DEFAULT_CONFIG.parse()?;
    Ok(defaults[list]
        .as_array_of_tables()
        .with_context(|| format!("no default {list}"))?
        .iter()
        .cloned()
        .collect())
}

/// `slug`, else `slug-2`, `slug-3`, ...: the first id not `taken`.
fn free_id(slug: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..)
        .map(|n| {
            if n == 1 {
                slug.to_string()
            } else {
                format!("{slug}-{n}")
            }
        })
        .find(|id| !taken(id))
        .expect("some suffix is free")
}

/// Append a click-only action that runs `prompt` (output = replace); returns
/// its id: the first three words as a slug (`-2`, `-3`, ... when taken, also
/// by a built-in action the file left out: that id could never be removed).
pub fn add_action_to(doc: &mut toml_edit::DocumentMut, prompt: &str) -> Result<String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        anyhow::bail!("the instruction is empty");
    }
    let words: Vec<&str> = prompt.split_whitespace().take(3).collect();
    let slug: String = words
        .iter()
        .flat_map(|w| w.split(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|r| !r.is_empty())
        .map(|r| r.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(24)
        .collect();
    let slug = match slug.trim_end_matches('-') {
        "" => "custom".to_string(),
        s => s.to_string(),
    };
    let short: String = words.join(" ").chars().take(24).collect();
    let mut chars = short.trim_end().chars();
    let name: String = chars
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .chain(chars)
        .collect();

    let builtin = default_actions();
    let actions = list_of(doc, "actions", true)?;
    let id = free_id(&slug, |id| {
        builtin.iter().any(|a| a.id == id) || actions.iter().any(|t| id_of(t) == Some(id))
    });
    let mut t = toml_edit::Table::new();
    t["id"] = toml_edit::value(&id);
    t["name"] = toml_edit::value(name);
    t["prompt"] = toml_edit::value(prompt);
    t["output"] = toml_edit::value("replace");
    actions.push(t);
    Ok(id)
}

/// Append a modifier for the user to fill in; returns its id (`custom`, `custom-2`, ...).
pub fn add_modifier_to(doc: &mut toml_edit::DocumentMut) -> Result<String> {
    let builtin = default_modifiers();
    let mods = list_of(doc, "modifiers", true)?;
    let id = free_id("custom", |id| {
        builtin.iter().any(|m| m.id == id) || mods.iter().any(|t| id_of(t) == Some(id))
    });
    let mut t = toml_edit::Table::new();
    t["id"] = toml_edit::value(&id);
    t["name"] = toml_edit::value("New modifier");
    t["suffix"] = toml_edit::value("Use simpler words.");
    mods.push(t);
    Ok(id)
}

/// The fields of an entry the popup's settings edit.
fn fields(list: &str) -> &'static [&'static str] {
    match list {
        "actions" => &["name", "key", "prompt", "output", "backend", "model"],
        _ => &["name", "key", "suffix"],
    }
}

/// Set fields of the entry `id` in `list` ("actions" or "modifiers"),
/// copying the built-in list into the file first if needed. An empty `key`,
/// `backend` or `model` removes it; a key is one character the popup does
/// not reserve. Duplicate keys are caught when the file is checked
/// ([`Config::edit_file`]).
pub fn patch_entry(
    doc: &mut toml_edit::DocumentMut,
    list: &str,
    id: &str,
    patch: &BTreeMap<String, String>,
) -> Result<()> {
    let aot = list_of(doc, list, true)?;
    let i = index_of(aot, list, id)?;
    let t = aot.get_mut(i).expect("index_of found it");
    for (field, value) in patch {
        let field = field.as_str();
        if !fields(list).contains(&field) {
            anyhow::bail!("{list} have no `{field}` to set here");
        }
        let mut value = value.clone();
        if field != "prompt" && field != "suffix" {
            value = value.trim().to_string();
        }
        if field == "key" {
            value = value.to_lowercase();
            if value.chars().count() > 1 {
                anyhow::bail!("a key is a single character");
            }
            if RESERVED_KEYS.contains(&value.as_str()) {
                anyhow::bail!("key \"{value}\" is reserved by the popup");
            }
        }
        match field {
            "key" | "backend" | "model" if value.is_empty() => {
                t.remove(field);
            }
            _ if value.trim().is_empty() => anyhow::bail!("`{field}` cannot be empty"),
            _ => put(t, field, value.into()),
        }
    }
    Ok(())
}

/// Move the entry `id` `delta` places in `list` (the order of the chips),
/// stopping at either end. Each place keeps its comments.
pub fn move_entry(
    doc: &mut toml_edit::DocumentMut,
    list: &str,
    id: &str,
    delta: isize,
) -> Result<()> {
    let aot = list_of(doc, list, true)?;
    let i = index_of(aot, list, id)?;
    let j = i.saturating_add_signed(delta).min(aot.len() - 1);
    if i == j {
        return Ok(());
    }
    let places: Vec<_> = aot
        .iter()
        .map(|t| (t.position(), t.decor().clone()))
        .collect();
    let mut tables: Vec<toml_edit::Table> = aot.iter().cloned().collect();
    let t = tables.remove(i);
    tables.insert(j, t);
    aot.clear();
    for (mut t, (pos, decor)) in tables.into_iter().zip(places) {
        if let Some(p) = pos {
            t.set_position(p);
        }
        *t.decor_mut() = decor;
        aot.push(t);
    }
    Ok(())
}

/// Give the built-in entry `id` its default name, key, prompt / suffix and
/// output back (an action keeps its `backend` and `model`).
pub fn reset_entry(doc: &mut toml_edit::DocumentMut, list: &str, id: &str) -> Result<()> {
    let default = default_entries(list)?
        .into_iter()
        .find(|t| id_of(t) == Some(id))
        .with_context(|| format!("`{id}` is not a built-in {}", noun(list)))?;
    let aot = list_of(doc, list, true)?;
    let i = index_of(aot, list, id)?;
    let t = aot.get_mut(i).expect("index_of found it");
    for f in fields(list) {
        if matches!(*f, "backend" | "model") {
            continue;
        }
        match default.get(f).and_then(|i| i.as_value()) {
            Some(v) => put(t, f, v.clone()),
            None => {
                t.remove(f);
            }
        }
    }
    Ok(())
}

/// Remove the entry `id` from the file's `list`. Built-in entries stay (edit
/// the file to drop those).
pub fn remove_entry(doc: &mut toml_edit::DocumentMut, list: &str, id: &str) -> Result<()> {
    if default_entries(list)?.iter().any(|t| id_of(t) == Some(id)) {
        anyhow::bail!(
            "`{id}` is a built-in {}; it cannot be removed here",
            noun(list)
        );
    }
    let not_found = || format!("no {} `{id}` in the config file", noun(list));
    let aot = list_of(doc, list, false).with_context(not_found)?;
    let i = index_of(aot, list, id)?;
    aot.remove(i);
    Ok(())
}

/// Remove the action `id` from the file. Built-in actions and the default
/// action stay (edit the file to drop those).
pub fn remove_action_from(
    doc: &mut toml_edit::DocumentMut,
    id: &str,
    default_action: &str,
) -> Result<()> {
    if id == default_action {
        anyhow::bail!("`{id}` is general.default_action; it cannot be removed");
    }
    remove_entry(doc, "actions", id)
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
    fn example_matches_defaults() {
        let c = Config::default();
        assert_eq!(c.general, General::default());
        assert_eq!(c.provider, Provider::default());
        assert_eq!(c.linux, Linux::default());
        assert_eq!(c.translate, Translate::default());
        assert!(c.system_prompt.contains("writing assistant"));
    }

    #[test]
    fn partial_config_fills_in_defaults() {
        let c: Config =
            toml::from_str("[provider]\nuse = \"deepseek\"\n[provider.models]\nollama = \"x\"\n")
                .unwrap();
        assert_eq!(c.provider.use_backend, "deepseek");
        assert_eq!(c.provider.models["ollama"], "x");
        assert_eq!(c.provider.order, Provider::default().order);
        assert_eq!(c.general, General::default());
        assert!(!c.actions.is_empty());
        assert!(!c.system_prompt.is_empty());

        let empty: Config = toml::from_str("").unwrap();
        assert_eq!(empty.actions.len(), Config::default().actions.len());
    }

    #[test]
    fn set_value_keeps_comments_and_creates_tables() {
        let out = set_toml_value(DEFAULT_CONFIG, &["provider", "use"], "claude-code").unwrap();
        let c: Config = toml::from_str(&out).unwrap();
        assert_eq!(c.provider.use_backend, "claude-code");
        assert!(out.contains("# mirzabenevis configuration"));
        assert_eq!(out.lines().count(), DEFAULT_CONFIG.lines().count());

        let out = set_toml_value(&out, &["provider", "models", "claude-code"], "sonnet").unwrap();
        let c: Config = toml::from_str(&out).unwrap();
        assert_eq!(c.provider.models["claude-code"], "sonnet");

        let out = set_toml_value("", &["provider", "models", "ollama"], "qwen3:8b").unwrap();
        let c: Config = toml::from_str(&out).unwrap();
        assert_eq!(c.provider.models["ollama"], "qwen3:8b");
    }

    #[test]
    fn add_and_remove_actions_keep_comments() {
        // A file without [[actions]] first gets the built-in ones.
        let head = "# mine\n[provider]\nuse = \"ollama\" # pinned\n";
        let mut doc: toml_edit::DocumentMut = head.parse().unwrap();
        let add = |doc: &mut toml_edit::DocumentMut, p| add_action_to(doc, p).unwrap();
        assert_eq!(
            add(&mut doc, "  make it sound like a pirate "),
            "make-it-sound"
        );
        assert_eq!(add(&mut doc, "Make it SOUND, please"), "make-it-sound-2");
        assert_eq!(add(&mut doc, "به فارسی بنویس"), "custom");
        assert!(add_action_to(&mut doc, " ").is_err());
        let out = doc.to_string();
        assert!(out.starts_with(head), "{out}");
        assert!(out.contains("# Actions appear in the popup"), "{out}");
        let c = Config::from_user_toml(&out).unwrap();
        let n = default_actions().len();
        assert_eq!(c.actions.len(), n + 3);
        let a = &c.actions[n];
        assert_eq!(
            (a.name.as_str(), a.prompt.as_str(), a.key.as_str(), a.output),
            (
                "Make it sound",
                "make it sound like a pirate",
                "",
                Output::Replace
            )
        );
        assert_eq!(c.actions[n + 1].name, "Make it SOUND,");
        assert_eq!(c.actions[n + 2].name, "به فارسی بنویس");

        // Add then remove on a file with its own actions: byte for byte the same.
        let mut doc: toml_edit::DocumentMut = DEFAULT_CONFIG.parse().unwrap();
        let id = add(&mut doc, "Be nice");
        assert_eq!(id, "be-nice");
        assert_ne!(doc.to_string(), DEFAULT_CONFIG);
        remove_action_from(&mut doc, &id, "fix").unwrap();
        assert_eq!(doc.to_string(), DEFAULT_CONFIG);

        // A built-in id is taken even when the file's own actions leave it out.
        let mut own: toml_edit::DocumentMut =
            "[[actions]]\nid = \"a\"\nname = \"A\"\nprompt = \"p\"\n"
                .parse()
                .unwrap();
        assert_eq!(add(&mut own, "Table"), "table-2");

        // Missing, built-in and default actions are refused.
        let id = add(&mut doc, "y");
        for (id, default) in [
            ("nope", "fix"),
            ("fix", "fix"),
            ("improve", "fix"),
            (&id, &id),
        ] {
            assert!(remove_action_from(&mut doc, id, default).is_err(), "{id}");
        }
        let mut empty = toml_edit::DocumentMut::new();
        assert!(remove_action_from(&mut empty, "nope", "fix").is_err());
    }

    #[test]
    fn typed_values_keep_their_type_and_comments() {
        let mut out = DEFAULT_CONFIG.to_string();
        let json = |s: &str| toml_value(&serde_json::from_str(s).unwrap()).unwrap();
        for (path, v) in [
            (&["general", "auto_paste"][..], "false"),
            (&["provider", "max_tokens"], "2048"),
            (&["provider", "temperature"], "1"),
            (&["translate", "targets"], r#"["en", "fr"]"#),
        ] {
            out = set_toml_value(&out, path, json(v)).unwrap();
        }
        assert_eq!(out.lines().count(), DEFAULT_CONFIG.lines().count());
        assert!(out.contains("# Replace the selection with the result automatically"));
        assert!(out.contains("\nauto_paste = false\n"), "{out}");
        assert!(out.contains("\nmax_tokens = 2048\n"), "{out}");
        let c = Config::from_user_toml(&out).unwrap();
        assert!(!c.general.auto_paste);
        assert_eq!(c.provider.max_tokens, 2048);
        assert_eq!(c.provider.temperature, 1.0);
        assert_eq!(c.translate.targets, ["en", "fr"]);
        assert!(toml_value(&serde_json::json!({"a": 1})).is_err());
        assert!(toml_value(&serde_json::json!([1])).is_err());
        // A string where a number belongs does not load.
        let bad = set_toml_value(&out, &["provider", "max_tokens"], "many").unwrap();
        assert!(Config::from_user_toml(&bad).is_err());
    }

    #[test]
    fn edit_entries_in_place() {
        let patch = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // No [[actions]] in the file: the built-in ones are copied in first.
        let head = "# mine\n[general]\nauto_paste = false # off\n";
        let mut doc: toml_edit::DocumentMut = head.parse().unwrap();
        let p = patch(&[("name", " Polish "), ("key", "9"), ("backend", "ollama")]);
        patch_entry(&mut doc, "actions", "improve", &p).unwrap();
        let out = doc.to_string();
        assert!(out.starts_with(head), "{out}");
        let c = Config::from_user_toml(&out).unwrap();
        assert_eq!(c.actions.len(), default_actions().len());
        let a = c.action("improve").unwrap();
        assert_eq!((a.name.as_str(), a.key.as_str()), ("Polish", "9"));
        assert_eq!(a.backend.as_deref(), Some("ollama"));
        patch_entry(&mut doc, "actions", "improve", &patch(&[("backend", "")])).unwrap();
        let c = Config::from_user_toml(&doc.to_string()).unwrap();
        assert_eq!(c.action("improve").unwrap().backend, None);

        // Reserved, long and empty values are refused with a message; a
        // duplicate key fails the check the file must pass before it is saved.
        for (p, msg) in [
            (patch(&[("key", "R")]), "key \"r\" is reserved by the popup"),
            (patch(&[("key", "ab")]), "a key is a single character"),
            (patch(&[("name", " ")]), "`name` cannot be empty"),
            (patch(&[("id", "x")]), "actions have no `id` to set here"),
        ] {
            let e = patch_entry(&mut doc, "actions", "fix", &p).unwrap_err();
            assert_eq!(e.to_string(), msg);
        }
        assert!(patch_entry(&mut doc, "actions", "nope", &patch(&[])).is_err());
        let mut dup = doc.clone();
        patch_entry(&mut dup, "actions", "fix", &patch(&[("key", "3")])).unwrap();
        let e = Config::from_user_toml(&dup.to_string()).unwrap_err();
        assert_eq!(e.to_string(), "duplicate action key \"3\"");

        // Reset puts the defaults back; backend and model stay.
        let p = patch(&[("prompt", "x"), ("output", "show"), ("model", "m")]);
        patch_entry(&mut doc, "actions", "fix", &p).unwrap();
        reset_entry(&mut doc, "actions", "fix").unwrap();
        reset_entry(&mut doc, "actions", "improve").unwrap();
        let c = Config::from_user_toml(&doc.to_string()).unwrap();
        let (fix, def) = (c.action("fix").unwrap(), &default_actions()[0]);
        assert_eq!(
            (&fix.name, &fix.key, &fix.prompt, fix.output),
            (&def.name, &def.key, &def.prompt, def.output)
        );
        assert_eq!(fix.model.as_deref(), Some("m"));
        assert_eq!(c.action("improve").unwrap().name, "Improve");
        let id = add_action_to(&mut doc, "Be nice").unwrap();
        assert!(reset_entry(&mut doc, "actions", &id).is_err());

        // A built-in modifier copied into the file loses a key your action uses.
        let mut doc: toml_edit::DocumentMut =
            "[[actions]]\nid = \"a\"\nname = \"A\"\nkey = \"w\"\nprompt = \"p\"\n"
                .parse()
                .unwrap();
        patch_entry(
            &mut doc,
            "modifiers",
            "shorter",
            &patch(&[("name", "Short")]),
        )
        .unwrap();
        let c = Config::from_user_toml(&doc.to_string()).unwrap();
        assert_eq!(c.modifiers[0].name, "Short");
        assert_eq!(
            c.modifiers.iter().find(|m| m.id == "warmer").unwrap().key,
            ""
        );
        let id = add_modifier_to(&mut doc).unwrap();
        assert_eq!(id, "custom");
        assert_eq!(add_modifier_to(&mut doc).unwrap(), "custom-2");
        let c = Config::from_user_toml(&doc.to_string()).unwrap();
        assert_eq!(c.modifiers.len(), default_modifiers().len() + 2);
        remove_entry(&mut doc, "modifiers", &id).unwrap();
        assert!(remove_entry(&mut doc, "modifiers", "warmer").is_err());
    }

    #[test]
    fn move_entries_round_trip() {
        let mut doc: toml_edit::DocumentMut = DEFAULT_CONFIG.parse().unwrap();
        let ids = |doc: &toml_edit::DocumentMut| -> Vec<String> {
            let c = Config::from_user_toml(&doc.to_string()).unwrap();
            c.actions.into_iter().map(|a| a.id).collect()
        };
        move_entry(&mut doc, "actions", "fix", 1).unwrap();
        assert_eq!(ids(&doc)[..3], ["improve", "fix", "concise"]);
        move_entry(&mut doc, "actions", "fix", -1).unwrap();
        assert_eq!(doc.to_string(), DEFAULT_CONFIG);
        // Clamped at the ends; a long move shifts the rest up.
        move_entry(&mut doc, "actions", "fix", -5).unwrap();
        assert_eq!(doc.to_string(), DEFAULT_CONFIG);
        move_entry(&mut doc, "actions", "fix", 99).unwrap();
        assert_eq!(ids(&doc).last().unwrap(), "fix");
        assert_eq!(ids(&doc)[0], "improve");
        move_entry(&mut doc, "actions", "fix", -99).unwrap();
        assert_eq!(doc.to_string(), DEFAULT_CONFIG);
    }

    #[test]
    fn render_substitutes_text() {
        let a = Action {
            prompt: "Fix: {text}".into(),
            ..Default::default()
        };
        assert_eq!(a.render("hi"), "Fix: <text>\nhi\n</text>");
        let a = Action {
            prompt: "Fix.\n".into(),
            ..Default::default()
        };
        assert_eq!(a.render("hi"), "Fix.\n\n<text>\nhi\n</text>");
        assert_eq!(
            a.render("a</text>b"),
            "Fix.\n\n<text>\na<\\/text>b\n</text>"
        );
    }

    #[test]
    fn modifiers_and_context_go_before_the_text() {
        let c = Config::default();
        let mods = [
            "warmer".to_string(),
            "shorter".to_string(),
            "nope".to_string(),
        ];
        let extra = c.extras(&mods, "  reply to my PI ");
        assert_eq!(
            extra,
            "Make it shorter.\nMake the tone warmer and friendlier.\nContext: reply to my PI"
        );
        assert_eq!(c.extras(&[], " "), "");
        let a = Action {
            prompt: "Fix.\n\n{text}".into(),
            ..Default::default()
        };
        // A {text} in the context is not expanded; only the first placeholder is.
        assert_eq!(
            a.render_with("hi", "Context: {text}"),
            "Fix.\n\nContext: {text}\n\n<text>\nhi\n</text>"
        );
        let a = Action {
            prompt: "Fix.\n".into(),
            ..Default::default()
        };
        assert_eq!(a.render_with("hi", "X"), "Fix.\n\nX\n\n<text>\nhi\n</text>");
    }

    #[test]
    fn duplicate_action_ids_and_keys_rejected() {
        assert!(Config::default().check_keys(false).is_ok());
        let mut c: Config = toml::from_str(
            "[[actions]]\nid = \"a\"\nname = \"A\"\nkey = \"1\"\nprompt = \"p\"\n\
             [[actions]]\nid = \"a\"\nname = \"B\"\nkey = \"1\"\nprompt = \"p\"\n\
             [[actions]]\nid = \"b\"\nname = \"C\"\nprompt = \"p\"\n\
             [[actions]]\nid = \"c\"\nname = \"D\"\nprompt = \"p\"\n",
        )
        .unwrap();
        let e = c.check_keys(false).unwrap_err().to_string();
        assert_eq!(e, "duplicate action id \"a\", key \"1\"");

        // Your action on a built-in modifier's key wins; reserved keys are dropped.
        let act = "[[actions]]\nid = \"a\"\nname = \"A\"\nkey = \"W\"\nprompt = \"p\"\n\
                   [[actions]]\nid = \"b\"\nname = \"B\"\nkey = \"R\"\nprompt = \"p\"\n";
        let c = Config::from_user_toml(act).unwrap();
        assert_eq!(
            (c.actions[0].key.as_str(), c.actions[1].key.as_str()),
            ("w", "")
        );
        assert_eq!(
            c.modifiers.iter().find(|m| m.id == "warmer").unwrap().key,
            ""
        );
        // ...but not your own modifier.
        let own =
            format!("{act}[[modifiers]]\nid = \"m\"\nname = \"M\"\nkey = \"w\"\nsuffix = \"s\"\n");
        let e = Config::from_user_toml(&own).unwrap_err().to_string();
        assert_eq!(e, "duplicate action key \"w\"");
    }
}
