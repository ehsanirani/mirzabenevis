//! Finding AI backends on this machine without any configuration.
//!
//! Three kinds are detected:
//! - **APIs**, when an API key is found: in the environment, in your login
//!   shell's environment, in `llm keys` (`keys.json`) or in opencode's `auth.json`;
//! - **agent CLIs** you are already logged in to: Claude Code (`claude`), Codex,
//!   Gemini CLI, `llm`;
//! - **local model servers** that are running: Ollama, LM Studio, llama.cpp, vLLM, Jan.
//!
//! [`detect`] returns everything found (sorted by `provider.order`) plus what was
//! checked and not found, with a hint. [`Detection::pick`] applies `provider.use`.

use crate::config::{Config, ProviderKind};
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A string that never shows up in logs or `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Api,
    Agent,
    Local,
    Custom,
}

/// A backend that can be used right now.
#[derive(Debug, Clone, Serialize)]
pub struct Backend {
    /// Stable id used in the config, e.g. `deepseek`, `claude-code`, `ollama`.
    pub id: String,
    pub label: String,
    pub category: Category,
    /// Model to request; empty means "the tool's own default".
    pub model: String,
    /// Where it was found, e.g. `DEEPSEEK_API_KEY (environment)`. Never a secret.
    pub source: String,
    #[serde(skip)]
    pub transport: Transport,
}

impl Backend {
    pub fn model_label(&self) -> &str {
        if self.model.is_empty() {
            "default model"
        } else {
            &self.model
        }
    }
}

#[derive(Debug, Clone)]
pub enum Transport {
    OpenAi(OpenAiWire),
    Anthropic {
        base_url: String,
        api_key: Secret,
        /// Talk to it directly, never through an HTTP proxy.
        local: bool,
    },
    /// Claude Code in print mode, using the user's own Claude login.
    ClaudeCode {
        bin: PathBuf,
        env: ChildEnv,
    },
    /// Any program that reads the prompt on stdin and prints the answer.
    /// `{system}` in `argv` is replaced by the system prompt; without it the
    /// system prompt is sent at the top of stdin.
    Command {
        argv: Vec<String>,
        env: ChildEnv,
    },
}

/// Extra environment for agent CLIs: the full PATH plus the variables only the
/// login shell knew (e.g. `OPENAI_API_KEY` from `~/.zshrc` for codex).
#[derive(Clone, Default)]
pub struct ChildEnv(pub Vec<(String, String)>);

impl fmt::Debug for ChildEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.0.iter().map(|(k, _)| k.as_str()).collect();
        write!(f, "ChildEnv({names:?})")
    }
}

/// How to talk to one OpenAI-compatible `/chat/completions` endpoint.
#[derive(Debug, Clone)]
pub struct OpenAiWire {
    pub base_url: String,
    pub api_key: Option<Secret>,
    /// Extra JSON merged into the request, e.g. switching reasoning off.
    pub extra: Value,
    /// Reasoning models reject `temperature`.
    pub temperature: bool,
    /// `max_tokens`, or `max_completion_tokens` for OpenAI's reasoning models.
    pub max_tokens_field: &'static str,
    /// Talk to it directly, never through an HTTP proxy.
    pub local: bool,
}

/// Something that was checked and is not usable, with what would make it work.
#[derive(Debug, Clone, Serialize)]
pub struct Missing {
    pub id: String,
    pub label: String,
    pub category: Category,
    pub hint: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Detection {
    pub found: Vec<Backend>,
    pub missing: Vec<Missing>,
    /// Whether the login shell was asked for its environment.
    pub used_login_shell: bool,
}

/// Never picked by `"auto"`, only when pinned: Gemini CLI keeps its read-only
/// tools, which untrusted text could steer.
const PINNED_ONLY: &[&str] = &["gemini-cli"];

impl Detection {
    /// What `"auto"` may use, in order.
    pub fn auto_candidates(&self) -> impl Iterator<Item = &Backend> {
        self.found
            .iter()
            .filter(|b| !PINNED_ONLY.contains(&b.id.as_str()))
    }

    /// Apply `provider.use`: `"auto"` (or empty) takes the first backend found.
    pub fn pick(&self, wanted: &str) -> Result<&Backend> {
        let wanted = wanted.trim();
        if wanted.is_empty() || wanted == "auto" {
            return self.auto_candidates().next().ok_or_else(|| {
                anyhow!(
                    "no AI backend found. Set an API key (e.g. DEEPSEEK_API_KEY or \
                     ANTHROPIC_API_KEY), install and log in to Claude Code (`claude`), \
                     or start Ollama. `mirzabenevis-cli providers` shows what was checked."
                )
            });
        }
        if let Some(b) = self.found.iter().find(|b| b.id == wanted) {
            return Ok(b);
        }
        match self.missing.iter().find(|m| m.id == wanted) {
            Some(m) => Err(anyhow!("{} is not available: {}", m.label, m.hint)),
            None => Err(anyhow!(
                "unknown backend `{wanted}` (see `mirzabenevis-cli providers`)"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Wire {
    OpenAi {
        temperature: bool,
        max_tokens_field: &'static str,
        /// JSON object merged into the request body.
        extra: &'static str,
    },
    Anthropic,
}

struct Api {
    id: &'static str,
    label: &'static str,
    env: &'static [&'static str],
    /// Key names in `llm keys` (`keys.json`).
    llm_keys: &'static [&'static str],
    /// Provider id in opencode's `auth.json`.
    opencode: &'static str,
    base_url: &'static str,
    model: &'static str,
    wire: Wire,
}

const fn oai(temperature: bool, max_tokens_field: &'static str, extra: &'static str) -> Wire {
    Wire::OpenAi {
        temperature,
        max_tokens_field,
        extra,
    }
}

// Fast, cheap, non-reasoning defaults, checked against provider docs in Nov 2026.
// Most "fast" models now think by default, which makes rewrites slow; `extra`
// switches that off where the provider allows it.
const APIS: &[Api] = &[
    Api {
        id: "anthropic",
        label: "Anthropic API",
        env: &["ANTHROPIC_API_KEY"],
        llm_keys: &["anthropic", "claude"],
        opencode: "anthropic",
        base_url: "https://api.anthropic.com",
        model: "claude-haiku-5-5",
        wire: Wire::Anthropic,
    },
    Api {
        id: "deepseek",
        label: "DeepSeek",
        env: &["DEEPSEEK_API_KEY"],
        llm_keys: &["deepseek"],
        opencode: "deepseek",
        base_url: "https://api.deepseek.com",
        model: "deepseek-flash",
        wire: oai(true, "max_tokens", r#"{"thinking":{"type":"disabled"}}"#),
    },
    Api {
        id: "openai",
        label: "OpenAI",
        env: &["OPENAI_API_KEY"],
        llm_keys: &["openai"],
        opencode: "openai",
        base_url: "https://api.openai.com/v1",
        model: "gpt-6-luna",
        wire: oai(
            false,
            "max_completion_tokens",
            r#"{"reasoning_effort":"none"}"#,
        ),
    },
    Api {
        id: "gemini",
        label: "Google Gemini",
        env: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        llm_keys: &["gemini"],
        opencode: "google",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        model: "gemini-3.5-flash-lite",
        wire: oai(false, "max_tokens", "{}"),
    },
    Api {
        id: "openrouter",
        label: "OpenRouter",
        env: &["OPENROUTER_API_KEY"],
        llm_keys: &["openrouter"],
        opencode: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        model: "openai/gpt-6-luna",
        wire: oai(false, "max_tokens", "{}"),
    },
    Api {
        id: "groq",
        label: "Groq",
        env: &["GROQ_API_KEY"],
        llm_keys: &["groq"],
        opencode: "groq",
        base_url: "https://api.groq.com/openai/v1",
        model: "llama-3.3-70b-versatile",
        wire: oai(true, "max_tokens", "{}"),
    },
    Api {
        id: "mistral",
        label: "Mistral",
        env: &["MISTRAL_API_KEY"],
        llm_keys: &["mistral"],
        opencode: "mistral",
        base_url: "https://api.mistral.ai/v1",
        model: "mistral-small-latest",
        wire: oai(true, "max_tokens", "{}"),
    },
    Api {
        id: "xai",
        label: "xAI Grok",
        env: &["XAI_API_KEY"],
        llm_keys: &["grok", "xai"],
        opencode: "xai",
        base_url: "https://api.x.ai/v1",
        model: "grok-4.20-non-reasoning",
        wire: oai(true, "max_tokens", "{}"),
    },
    Api {
        id: "qwen",
        label: "Qwen (DashScope)",
        env: &["DASHSCOPE_API_KEY"],
        llm_keys: &["dashscope", "qwen"],
        opencode: "alibaba",
        base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        model: "qwen-flash",
        wire: oai(true, "max_tokens", r#"{"enable_thinking":false}"#),
    },
    Api {
        id: "together",
        label: "Together AI",
        env: &["TOGETHER_API_KEY"],
        llm_keys: &["together"],
        opencode: "togetherai",
        base_url: "https://api.together.ai/v1",
        model: "Qwen/Qwen3.8-Flash",
        wire: oai(true, "max_tokens", "{}"),
    },
    Api {
        id: "cerebras",
        label: "Cerebras",
        env: &["CEREBRAS_API_KEY"],
        llm_keys: &["cerebras"],
        opencode: "cerebras",
        base_url: "https://api.cerebras.ai/v1",
        model: "gpt-oss-120b",
        wire: oai(false, "max_tokens", r#"{"reasoning_effort":"low"}"#),
    },
    Api {
        id: "moonshot",
        label: "Moonshot Kimi",
        env: &["MOONSHOT_API_KEY"],
        llm_keys: &["moonshot", "kimi"],
        opencode: "moonshotai",
        base_url: "https://api.moonshot.ai/v1",
        model: "kimi-k3.1-pro-highspeed",
        wire: oai(false, "max_tokens", r#"{"reasoning_effort":"low"}"#),
    },
];

enum AgentKind {
    ClaudeCode,
    /// `bin head.. [model_flag model] tail..`
    Command {
        head: &'static [&'static str],
        model_flag: &'static str,
        tail: &'static [&'static str],
    },
}

struct Agent {
    id: &'static str,
    label: &'static str,
    bin: &'static str,
    /// Empty: let the tool use its own default.
    model: &'static str,
    kind: AgentKind,
}

const AGENTS: &[Agent] = &[
    Agent {
        id: "claude-code",
        label: "Claude Code",
        bin: "claude",
        model: "haiku",
        kind: AgentKind::ClaudeCode,
    },
    Agent {
        id: "codex",
        label: "Codex CLI",
        bin: "codex",
        model: "",
        kind: AgentKind::Command {
            head: &[
                "exec",
                "--skip-git-repo-check",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "-c",
                "features.shell_tool=false",
                "-c",
                "web_search=\"disabled\"",
            ],
            model_flag: "-m",
            tail: &["-"],
        },
    },
    Agent {
        id: "gemini-cli",
        label: "Gemini CLI",
        bin: "gemini",
        model: "",
        kind: AgentKind::Command {
            head: &["-o", "text", "--approval-mode", "plan"],
            model_flag: "-m",
            tail: &["-p", "Follow the instructions in the text above."],
        },
    },
    Agent {
        id: "llm",
        label: "llm",
        bin: "llm",
        model: "",
        kind: AgentKind::Command {
            head: &["-n", "-s", "{system}"],
            model_flag: "-m",
            tail: &[],
        },
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelList {
    /// Ollama's native `GET /api/tags`.
    OllamaTags,
    /// `GET /v1/models`.
    OpenAi,
}

struct Local {
    id: &'static str,
    label: &'static str,
    /// Server roots to probe, without `/v1`.
    bases: &'static [&'static str],
    list: ModelList,
}

const LOCALS: &[Local] = &[
    Local {
        id: "ollama",
        label: "Ollama",
        bases: &["http://127.0.0.1:11434"],
        list: ModelList::OllamaTags,
    },
    Local {
        id: "lmstudio",
        label: "LM Studio",
        bases: &["http://127.0.0.1:1234"],
        list: ModelList::OpenAi,
    },
    Local {
        id: "llamacpp",
        label: "llama.cpp server",
        bases: &["http://127.0.0.1:8080", "http://127.0.0.1:9931"],
        list: ModelList::OpenAi,
    },
    Local {
        id: "vllm",
        label: "vLLM",
        bases: &["http://127.0.0.1:8000"],
        list: ModelList::OpenAi,
    },
    Local {
        id: "jan",
        label: "Jan",
        bases: &["http://127.0.0.1:1337"],
        list: ModelList::OpenAi,
    },
];

/// Ids of every backend this build knows about, in catalog order.
pub fn known_ids() -> Vec<&'static str> {
    std::iter::once("custom")
        .chain(APIS.iter().map(|a| a.id))
        .chain(AGENTS.iter().map(|a| a.id))
        .chain(LOCALS.iter().map(|l| l.id))
        .collect()
}

// ---------------------------------------------------------------------------
// Environment: variables, PATH, other tools' key stores
// ---------------------------------------------------------------------------

/// Where to look for keys and programs. Built from the real system by
/// [`Env::gather`], or by hand in tests.
#[derive(Clone, Default)]
pub struct Env {
    vars: HashMap<String, String>,
    /// Variables the login shell had and this process lacks (or has empty).
    shell_vars: HashMap<String, String>,
    path: Vec<PathBuf>,
    home: Option<PathBuf>,
    llm_keys: HashMap<String, String>,
    opencode_keys: HashMap<String, String>,
    pub used_login_shell: bool,
}

// Hand-written so key values can never end up in a log line.
impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env")
            .field("vars", &self.vars.len())
            .field("shell_vars", &self.shell_vars.keys().collect::<Vec<_>>())
            .field("path", &self.path)
            .field("llm_keys", &self.llm_keys.keys().collect::<Vec<_>>())
            .field(
                "opencode_keys",
                &self.opencode_keys.keys().collect::<Vec<_>>(),
            )
            .field("used_login_shell", &self.used_login_shell)
            .finish()
    }
}

impl Env {
    /// The real environment. Asks the login shell too when `linux.login_shell_env`
    /// is on and we are not running inside a terminal (i.e. started by the
    /// desktop or a keybinding, which usually lack what `~/.zshrc` exports).
    ///
    /// The login shell's answer is cached for a while (in `$XDG_RUNTIME_DIR`,
    /// private to the user) so a keybinding does not start a shell every time;
    /// `fresh_shell` ignores the cache.
    pub async fn gather(cfg: &Config, fresh_shell: bool) -> Env {
        let vars: HashMap<String, String> = std::env::vars().collect();
        let in_terminal = std::io::stderr().is_terminal();
        let shell = if cfg.linux.login_shell_env && !in_terminal {
            shell_env_cached(vars.get("SHELL").map(String::as_str), fresh_shell).await
        } else {
            None
        };
        let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
        Env::build(vars, shell, home.as_deref())
    }

    fn build(
        vars: HashMap<String, String>,
        shell: Option<HashMap<String, String>>,
        home: Option<&Path>,
    ) -> Env {
        let used_login_shell = shell.is_some();
        let mut shell_vars = HashMap::new();
        let mut path = Vec::new();
        let push_path = |p: &str, path: &mut Vec<PathBuf>| {
            for dir in std::env::split_paths(p) {
                if !dir.as_os_str().is_empty() && !path.contains(&dir) {
                    path.push(dir);
                }
            }
        };
        if let Some(p) = vars.get("PATH") {
            push_path(p, &mut path);
        }
        if let Some(shell) = shell {
            if let Some(p) = shell.get("PATH") {
                push_path(p, &mut path);
            }
            for (k, v) in shell {
                let ours_is_empty = vars.get(&k).is_none_or(|v| v.trim().is_empty());
                if ours_is_empty && !v.trim().is_empty() {
                    shell_vars.insert(k, v);
                }
            }
        }
        // Where agent CLIs like to install themselves, in case PATH misses them.
        if let Some(home) = home {
            for rel in [
                ".local/bin",
                ".claude/local",
                ".npm-global/bin",
                ".bun/bin",
                ".cargo/bin",
                ".nix-profile/bin",
            ] {
                let d = home.join(rel);
                if !path.contains(&d) {
                    path.push(d);
                }
            }
        }
        if let Some(user) = vars.get("USER") {
            let d = PathBuf::from(format!("/etc/profiles/per-user/{user}/bin"));
            if !path.contains(&d) {
                path.push(d);
            }
        }
        for d in ["/run/current-system/sw/bin", "/usr/local/bin", "/usr/bin"] {
            let d = PathBuf::from(d);
            if !path.contains(&d) {
                path.push(d);
            }
        }

        let lookup = |k: &str| {
            vars.get(k)
                .or_else(|| shell_vars.get(k))
                .map(String::as_str)
        };
        let config_home = lookup("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")));
        let data_home = lookup("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share")));
        let llm_dir = lookup("LLM_USER_PATH")
            .map(PathBuf::from)
            .or_else(|| config_home.map(|c| c.join("io.datasette.llm")));
        let llm_keys = llm_dir
            .map(|d| read_llm_keys(&d.join("keys.json")))
            .unwrap_or_default();
        let opencode_keys = data_home
            .map(|d| read_opencode_keys(&d.join("opencode/auth.json")))
            .unwrap_or_default();

        Env {
            vars,
            shell_vars,
            path,
            home: home.map(Path::to_path_buf),
            llm_keys,
            opencode_keys,
            used_login_shell,
        }
    }

    /// For tests: explicit variables, PATH directories and key stores.
    pub fn for_test(
        vars: &[(&str, &str)],
        path: Vec<PathBuf>,
        llm_keys: &[(&str, &str)],
        opencode_keys: &[(&str, &str)],
    ) -> Env {
        let own = |kv: &[(&str, &str)]| {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>()
        };
        Env {
            vars: own(vars),
            shell_vars: HashMap::new(),
            path,
            home: None,
            llm_keys: own(llm_keys),
            opencode_keys: own(opencode_keys),
            used_login_shell: false,
        }
    }

    /// A non-empty variable, and whether it came from the login shell.
    fn var(&self, k: &str) -> Option<(&str, bool)> {
        let nonempty = |v: &String| !v.trim().is_empty();
        if let Some(v) = self.vars.get(k).filter(|v| nonempty(v)) {
            return Some((v.trim(), false));
        }
        self.shell_vars
            .get(k)
            .filter(|v| nonempty(v))
            .map(|v| (v.trim(), true))
    }

    pub fn which(&self, bin: &str) -> Option<PathBuf> {
        self.path
            .iter()
            .map(|d| d.join(bin))
            .find(|p| is_executable(p))
    }

    /// PATH for child processes: what we know plus what the login shell knows,
    /// so e.g. `claude` (a Node script) can find `node`.
    pub fn path_string(&self) -> String {
        std::env::join_paths(&self.path)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Environment additions for agent CLIs.
    pub fn child_env(&self) -> ChildEnv {
        let mut env = vec![("PATH".to_string(), self.path_string())];
        env.extend(
            self.shell_vars
                .iter()
                .filter(|(k, _)| k.as_str() != "PATH")
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        env.sort();
        ChildEnv(env)
    }

    /// `~/x` → `$HOME/x`; a relative path with a slash → absolute (from the cwd).
    fn resolve_program(&self, prog: &str) -> Option<PathBuf> {
        if let Some(rest) = prog.strip_prefix("~/") {
            let p = self.home.as_ref()?.join(rest);
            return is_executable(&p).then_some(p);
        }
        if prog.contains('/') {
            let p = std::fs::canonicalize(prog).ok()?;
            return is_executable(&p).then_some(p);
        }
        self.which(prog)
    }
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `llm keys set openai` writes `{"openai": "sk-..."}`.
fn read_llm_keys(path: &Path) -> HashMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&raw) else {
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
        .filter(|(_, v)| !v.trim().is_empty())
        .collect()
}

/// opencode stores `{"deepseek": {"type": "api", "key": "..."}}`; OAuth
/// entries are skipped because they are not plain API keys.
fn read_opencode_keys(path: &Path) -> HashMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&raw) else {
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(k, v)| {
            if v["type"] != "api" {
                return None;
            }
            Some((k, v["key"].as_str()?.to_string()))
        })
        .filter(|(_, v)| !v.trim().is_empty())
        .collect()
}

/// Which login-shell variables matter: PATH, API keys and tokens, and what
/// agent CLIs or Node version managers read. Only these are kept and cached.
fn interesting_var(k: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "ANTHROPIC_",
        "CLAUDE_",
        "OPENAI_",
        "CODEX_",
        "GEMINI_",
        "GOOGLE_",
        "OLLAMA_",
        "LLM_",
        "XDG_",
        "NVM_",
        "NODE_",
        "VOLTA_",
        "BUN_",
        "PNPM_",
    ];
    const SUFFIXES: &[&str] = &["_API_KEY", "_TOKEN", "_BASE_URL"];
    const EXACT: &[&str] = &[
        "PATH",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
        "https_proxy",
        "http_proxy",
        "no_proxy",
    ];
    EXACT.contains(&k)
        || PREFIXES.iter().any(|p| k.starts_with(p))
        || SUFFIXES.iter().any(|s| k.ends_with(s))
}

fn shell_cache_file() -> Option<PathBuf> {
    // XDG_RUNTIME_DIR is per-user, 0700 and in RAM; without it, don't cache.
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(
        PathBuf::from(dir)
            .join("mirzabenevis")
            .join("shell-env.json"),
    )
}

const SHELL_CACHE_TTL: Duration = Duration::from_secs(30 * 60);

async fn shell_env_cached(shell: Option<&str>, fresh: bool) -> Option<HashMap<String, String>> {
    let cache = shell_cache_file();
    if let (false, Some(file)) = (fresh, &cache) {
        let young = std::fs::metadata(file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < SHELL_CACHE_TTL);
        if young {
            if let Some(map) = std::fs::read(file)
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
            {
                return Some(map);
            }
        }
    }
    let env = login_shell_env(shell).await?;
    if let Some(file) = &cache {
        write_private(file, &serde_json::to_vec(&env).unwrap_or_default());
    }
    Some(env)
}

/// Write a file only the user can read (it may hold API keys).
fn write_private(file: &Path, data: &[u8]) {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = file.parent() {
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir);
    }
    let tmp = file.with_extension("tmp");
    let ok = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(data));
    if ok.is_ok() {
        let _ = std::fs::rename(&tmp, file);
    }
}

/// Run the login shell interactively and print its environment. Interactive
/// so `~/.zshrc` is read; bash reads `~/.bashrc` only when not a login shell,
/// so it is sourced explicitly. A marker separates rc-file chatter from the
/// output. The shell runs in its own process group, killed on timeout, so
/// anything the rc files start does not linger.
async fn login_shell_env(shell: Option<&str>) -> Option<HashMap<String, String>> {
    const MARKER: &str = "__MIRZABENEVIS_ENV__";
    let shell = shell.filter(|s| !s.is_empty()).unwrap_or("/bin/sh");
    let is_bash = Path::new(shell).file_name().is_some_and(|n| n == "bash");
    let bashrc = if is_bash {
        "[ -f ~/.bashrc ] && . ~/.bashrc >/dev/null 2>&1; "
    } else {
        ""
    };
    let script = format!("{bashrc}printf '\\n{MARKER}\\n'; env -0");
    let mut cmd = tokio::process::Command::new(shell);
    cmd.args(["-l", "-i", "-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    let child = cmd.spawn().ok()?;
    let group = crate::provider::ProcessGroup::new(child.id());
    let out = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output()).await;
    drop(group);
    let out = match out {
        Ok(Ok(out)) => out.stdout,
        _ => {
            tracing::debug!("login shell environment unavailable");
            return None;
        }
    };
    let text = String::from_utf8_lossy(&out);
    let body = text.split_once(&format!("{MARKER}\n"))?.1;
    Some(
        body.split('\0')
            .filter_map(|kv| kv.split_once('='))
            .filter(|(k, _)| interesting_var(k))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect with the real environment.
/// `fresh_shell` re-reads the login shell instead of using the cached copy.
pub async fn detect_current(cfg: &Config, fresh_shell: bool) -> Detection {
    let env = Env::gather(cfg, fresh_shell).await;
    detect(cfg, &env).await
}

pub async fn detect(cfg: &Config, env: &Env) -> Detection {
    let locals = futures_util::future::join_all(LOCALS.iter().map(|l| probe_local(l, cfg, env)));
    detect_with_locals(cfg, env, locals.await)
}

fn detect_with_locals(
    cfg: &Config,
    env: &Env,
    locals: Vec<std::result::Result<Backend, Missing>>,
) -> Detection {
    let mut found = Vec::new();
    let mut missing = Vec::new();

    match custom_backend(cfg, env) {
        Some(Ok(b)) => found.push(b),
        Some(Err(m)) => missing.push(m),
        None => missing.push(Missing {
            id: "custom".into(),
            label: "Custom".into(),
            category: Category::Custom,
            hint: "not configured: set base_url or command in [provider.custom]".into(),
        }),
    }

    for api in APIS {
        match api_key(api, env) {
            Some((key, source)) => found.push(api_backend(api, cfg, key, source)),
            None => missing.push(Missing {
                id: api.id.into(),
                label: api.label.into(),
                category: Category::Api,
                hint: format!("set {}", api.env.join(" or ")),
            }),
        }
    }

    for agent in AGENTS {
        match env.which(agent.bin) {
            Some(bin) => found.push(agent_backend(agent, cfg, env, bin)),
            None => missing.push(Missing {
                id: agent.id.into(),
                label: agent.label.into(),
                category: Category::Agent,
                hint: format!("`{}` not found in PATH", agent.bin),
            }),
        }
    }

    for r in locals {
        match r {
            Ok(b) => found.push(b),
            Err(m) => missing.push(m),
        }
    }

    let rank = |id: &str| -> usize {
        let order = &cfg.provider.order;
        order.iter().position(|o| o == id).unwrap_or_else(|| {
            order.len()
                + known_ids()
                    .iter()
                    .position(|k| *k == id)
                    .unwrap_or(usize::MAX / 2)
        })
    };
    found.sort_by_key(|b| rank(&b.id));

    Detection {
        found,
        missing,
        used_login_shell: env.used_login_shell,
    }
}

fn model_for(cfg: &Config, id: &str, default: &str) -> String {
    cfg.provider
        .models
        .get(id)
        .filter(|m| !m.trim().is_empty())
        .map(|m| m.trim().to_string())
        .unwrap_or_else(|| default.to_string())
}

/// The key and a human-readable source. Never logs the key.
fn api_key(api: &Api, env: &Env) -> Option<(Secret, String)> {
    for var in api.env {
        if let Some((v, from_shell)) = env.var(var) {
            let origin = if from_shell {
                "login shell"
            } else {
                "environment"
            };
            return Some((Secret(v.to_string()), format!("{var} ({origin})")));
        }
    }
    for name in api.llm_keys {
        if let Some(v) = env.llm_keys.get(*name) {
            return Some((Secret(v.trim().to_string()), format!("llm keys: {name}")));
        }
    }
    if let Some(v) = env.opencode_keys.get(api.opencode) {
        return Some((
            Secret(v.trim().to_string()),
            format!("opencode auth: {}", api.opencode),
        ));
    }
    None
}

fn api_backend(api: &Api, cfg: &Config, key: Secret, source: String) -> Backend {
    let model = model_for(cfg, api.id, api.model);
    let transport = match api.wire {
        Wire::Anthropic => Transport::Anthropic {
            base_url: api.base_url.into(),
            api_key: key,
            local: false,
        },
        Wire::OpenAi {
            temperature,
            max_tokens_field,
            extra,
        } => Transport::OpenAi(OpenAiWire {
            base_url: api.base_url.into(),
            api_key: Some(key),
            extra: serde_json::from_str(extra).expect("catalog extra is valid JSON"),
            temperature,
            max_tokens_field,
            local: false,
        }),
    };
    Backend {
        id: api.id.into(),
        label: api.label.into(),
        category: Category::Api,
        model,
        source,
        transport,
    }
}

fn agent_backend(agent: &Agent, cfg: &Config, env: &Env, bin: PathBuf) -> Backend {
    let model = model_for(cfg, agent.id, agent.model);
    let source = bin.display().to_string();
    let transport = match agent.kind {
        AgentKind::ClaudeCode => Transport::ClaudeCode {
            bin,
            env: env.child_env(),
        },
        AgentKind::Command {
            head,
            model_flag,
            tail,
        } => {
            let mut argv = vec![bin.display().to_string()];
            argv.extend(head.iter().map(|s| s.to_string()));
            if !model.is_empty() {
                argv.push(model_flag.to_string());
                argv.push(model.clone());
            }
            argv.extend(tail.iter().map(|s| s.to_string()));
            Transport::Command {
                argv,
                env: env.child_env(),
            }
        }
    };
    Backend {
        id: agent.id.into(),
        label: agent.label.into(),
        category: Category::Agent,
        model,
        source,
        transport,
    }
}

fn is_local_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// `None` when the custom backend is not configured at all.
fn custom_backend(cfg: &Config, env: &Env) -> Option<std::result::Result<Backend, Missing>> {
    let c = &cfg.provider.custom;
    let missing = |hint: String| Missing {
        id: "custom".into(),
        label: "Custom".into(),
        category: Category::Custom,
        hint,
    };
    let model = model_for(cfg, "custom", &c.model);
    let key = if !c.api_key.trim().is_empty() {
        Some((Secret(c.api_key.trim().into()), "config file".to_string()))
    } else if !c.api_key_env.trim().is_empty() {
        env.var(c.api_key_env.trim()).map(|(v, from_shell)| {
            let origin = if from_shell {
                "login shell"
            } else {
                "environment"
            };
            (
                Secret(v.into()),
                format!("{} ({origin})", c.api_key_env.trim()),
            )
        })
    } else {
        None
    };

    let transport = match c.kind {
        ProviderKind::Command => {
            if c.command.is_empty() {
                return None;
            }
            let mut argv = c.command.clone();
            match env.resolve_program(&argv[0]) {
                Some(p) => argv[0] = p.display().to_string(),
                None => return Some(Err(missing(format!("`{}` not found", argv[0])))),
            }
            Transport::Command {
                argv,
                env: env.child_env(),
            }
        }
        ProviderKind::OpenAi => {
            if c.base_url.trim().is_empty() {
                return None;
            }
            if model.is_empty() {
                return Some(Err(missing("set model in [provider.custom]".into())));
            }
            Transport::OpenAi(OpenAiWire {
                base_url: c.base_url.trim().into(),
                api_key: key.as_ref().map(|(k, _)| k.clone()),
                extra: Value::Object(Default::default()),
                temperature: true,
                max_tokens_field: "max_tokens",
                local: is_local_url(&c.base_url),
            })
        }
        ProviderKind::Anthropic => {
            if c.base_url.trim().is_empty() {
                return None;
            }
            let Some((k, _)) = key.as_ref() else {
                return Some(Err(missing(
                    "kind = \"anthropic\" needs api_key or api_key_env".into(),
                )));
            };
            if model.is_empty() {
                return Some(Err(missing("set model in [provider.custom]".into())));
            }
            Transport::Anthropic {
                base_url: c.base_url.trim().into(),
                api_key: k.clone(),
                local: is_local_url(&c.base_url),
            }
        }
    };
    let source = match (&c.kind, &key) {
        (ProviderKind::Command, _) => c.command.join(" "),
        (_, Some((_, src))) => format!("{} (key: {src})", c.base_url.trim()),
        (_, None) => c.base_url.trim().to_string(),
    };
    Some(Ok(Backend {
        id: "custom".into(),
        label: "Custom".into(),
        category: Category::Custom,
        model,
        source,
        transport,
    }))
}

fn local_bases(l: &Local, env: &Env) -> Vec<String> {
    if l.id == "ollama" {
        if let Some(base) = env.var("OLLAMA_HOST").and_then(|(h, _)| ollama_base(h)) {
            return vec![base];
        }
    }
    l.bases.iter().map(|s| s.to_string()).collect()
}

/// Turn `OLLAMA_HOST` (as Ollama accepts it) into a URL to connect to:
/// `0.0.0.0` / `[::]` (listen everywhere) become loopback, a bare host gets
/// Ollama's port 11434, an explicit scheme keeps its default port.
fn ollama_base(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('/');
    let (with_scheme, explicit_scheme) = if raw.contains("://") {
        (raw.to_string(), true)
    } else if raw.starts_with(':') {
        (format!("http://127.0.0.1{raw}"), false)
    } else {
        (format!("http://{raw}"), false)
    };
    let mut url = reqwest::Url::parse(&with_scheme).ok()?;
    match url.host() {
        Some(url::Host::Ipv4(ip)) if ip.is_unspecified() => {
            url.set_host(Some("127.0.0.1")).ok()?;
        }
        Some(url::Host::Ipv6(ip)) if ip.is_unspecified() => {
            url.set_host(Some("[::1]")).ok()?;
        }
        None => return None,
        _ => {}
    }
    if url.port().is_none() && !explicit_scheme {
        url.set_port(Some(11434)).ok()?;
    }
    Some(url.as_str().trim_end_matches('/').to_string())
}

async fn probe_local(l: &Local, cfg: &Config, env: &Env) -> std::result::Result<Backend, Missing> {
    let missing = |hint: String| Missing {
        id: l.id.into(),
        label: l.label.into(),
        category: Category::Local,
        hint,
    };
    let bases = local_bases(l, env);
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(300))
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| missing(e.to_string()))?;
    for base in &bases {
        let url = match l.list {
            ModelList::OllamaTags => format!("{base}/api/tags"),
            ModelList::OpenAi => format!("{base}/v1/models"),
        };
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(missing(format!(
                "running at {base} but needs an API key: use [provider.custom]"
            )));
        }
        if !status.is_success() {
            continue;
        }
        let Ok(body) = resp.json::<Value>().await else {
            continue;
        };
        let models = parse_model_list(l.list, &body);
        let model = match cfg
            .provider
            .models
            .get(l.id)
            .filter(|m| !m.trim().is_empty())
        {
            Some(m) => m.trim().to_string(),
            None => match models.first() {
                Some(m) => m.clone(),
                None => {
                    return Err(missing(format!(
                        "running at {base} but no chat model is installed"
                    )))
                }
            },
        };
        return Ok(Backend {
            id: l.id.into(),
            label: l.label.into(),
            category: Category::Local,
            model,
            source: format!("{base} ({} models)", models.len()),
            transport: Transport::OpenAi(OpenAiWire {
                base_url: format!("{base}/v1"),
                api_key: None,
                extra: Value::Object(Default::default()),
                temperature: true,
                max_tokens_field: "max_tokens",
                local: true,
            }),
        });
    }
    Err(missing(format!(
        "not running (checked {})",
        bases.join(", ")
    )))
}

/// Model ids from a listing, skipping embedding and reranking models.
fn parse_model_list(kind: ModelList, body: &Value) -> Vec<String> {
    let items = match kind {
        ModelList::OllamaTags => body["models"].as_array(),
        ModelList::OpenAi => body["data"].as_array(),
    };
    let key = match kind {
        ModelList::OllamaTags => "name",
        ModelList::OpenAi => "id",
    };
    items
        .into_iter()
        .flatten()
        .filter(|m| !is_embedding_model(m, key))
        .filter_map(|m| m[key].as_str())
        .map(str::to_string)
        .collect()
}

fn is_embedding_model(m: &Value, key: &str) -> bool {
    const NAMES: &[&str] = &[
        "embed",
        "bge-",
        "bge:",
        "minilm",
        "rerank",
        "e5-",
        "gte-",
        "arctic-embed",
        "paraphrase-",
    ];
    let name = m[key].as_str().unwrap_or_default().to_lowercase();
    // Ollama reports BERT-family models, which are all embedding models.
    let bert = m["details"]["family"]
        .as_str()
        .is_some_and(|f| f.contains("bert"));
    bert || NAMES.iter().any(|n| name.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn no_locals() -> Vec<std::result::Result<Backend, Missing>> {
        Vec::new()
    }

    fn fake_bin(dir: &Path, name: &str) {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mzb-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn nothing_found_gives_helpful_error() {
        let cfg = Config::default();
        let env = Env::for_test(&[], vec![], &[], &[]);
        let d = detect_with_locals(&cfg, &env, no_locals());
        assert!(d.found.is_empty());
        let err = d.pick("auto").unwrap_err().to_string();
        assert!(err.contains("DEEPSEEK_API_KEY"), "{err}");
        assert!(d.missing.iter().any(|m| m.id == "claude-code"));
    }

    #[test]
    fn env_keys_are_found_and_ordered() {
        let cfg = Config::default();
        let env = Env::for_test(
            &[("DEEPSEEK_API_KEY", "sk-d"), ("ANTHROPIC_API_KEY", "sk-a")],
            vec![],
            &[],
            &[],
        );
        let d = detect_with_locals(&cfg, &env, no_locals());
        let ids: Vec<_> = d.found.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["anthropic", "deepseek"]);
        assert_eq!(d.pick("auto").unwrap().id, "anthropic");
        assert_eq!(d.pick("deepseek").unwrap().model, "deepseek-flash");
        assert!(d.found[1].source.contains("DEEPSEEK_API_KEY"));
        // The key never leaks through Debug.
        assert!(!format!("{:?}", d.found).contains("sk-d"));
    }

    #[test]
    fn empty_env_value_is_ignored_and_gemini_has_two_names() {
        let cfg = Config::default();
        let env = Env::for_test(
            &[("OPENAI_API_KEY", "  "), ("GOOGLE_API_KEY", "g")],
            vec![],
            &[],
            &[],
        );
        let d = detect_with_locals(&cfg, &env, no_locals());
        let ids: Vec<_> = d.found.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["gemini"]);
    }

    #[test]
    fn keys_from_llm_and_opencode() {
        let cfg = Config::default();
        let env = Env::for_test(&[], vec![], &[("deepseek", "k1")], &[("google", "k2")]);
        let d = detect_with_locals(&cfg, &env, no_locals());
        let ds = d.pick("deepseek").unwrap();
        assert_eq!(ds.source, "llm keys: deepseek");
        assert_eq!(d.pick("gemini").unwrap().source, "opencode auth: google");
    }

    #[test]
    fn key_stores_parse() {
        let dir = tmpdir("stores");
        let llm = dir.join("keys.json");
        std::fs::write(&llm, r#"{"// Note": "This file stores secret API credentials. Do not share!", "openai": "sk", "empty": ""}"#).unwrap();
        let keys = read_llm_keys(&llm);
        assert_eq!(keys.get("openai").map(String::as_str), Some("sk"));
        assert!(!keys.contains_key("empty"));
        let oc = dir.join("auth.json");
        std::fs::write(
            &oc,
            r#"{"deepseek":{"type":"api","key":"k"},"anthropic":{"type":"oauth","refresh":"r"}}"#,
        )
        .unwrap();
        let keys = read_opencode_keys(&oc);
        assert_eq!(keys.get("deepseek").map(String::as_str), Some("k"));
        assert!(!keys.contains_key("anthropic"));
        assert!(read_llm_keys(&dir.join("nope.json")).is_empty());
    }

    #[test]
    fn agent_clis_found_on_path_with_model_override() {
        let dir = tmpdir("agents");
        fake_bin(&dir, "claude");
        fake_bin(&dir, "llm");
        std::fs::write(dir.join("codex"), "not executable").unwrap();
        let mut cfg = Config::default();
        cfg.provider.models.insert("llm".into(), "gpt-x".into());
        let env = Env::for_test(&[], vec![dir.clone()], &[], &[]);
        let d = detect_with_locals(&cfg, &env, no_locals());
        let ids: Vec<_> = d.found.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["claude-code", "llm"]);
        assert_eq!(d.found[0].model, "haiku");
        match &d.found[1].transport {
            Transport::Command { argv, .. } => {
                assert!(argv[0].ends_with("/llm"));
                assert_eq!(&argv[1..], ["-n", "-s", "{system}", "-m", "gpt-x"]);
            }
            t => panic!("unexpected {t:?}"),
        }
    }

    #[test]
    fn pinned_backend_and_errors() {
        let mut cfg = Config::default();
        cfg.provider.order = vec!["deepseek".into()];
        let env = Env::for_test(
            &[("ANTHROPIC_API_KEY", "a"), ("DEEPSEEK_API_KEY", "d")],
            vec![],
            &[],
            &[],
        );
        let d = detect_with_locals(&cfg, &env, no_locals());
        // Listed ids first, then the rest in catalog order.
        assert_eq!(d.pick("auto").unwrap().id, "deepseek");
        assert_eq!(d.pick("anthropic").unwrap().id, "anthropic");
        let e = d.pick("groq").unwrap_err().to_string();
        assert!(e.contains("GROQ_API_KEY"), "{e}");
        let e = d.pick("nonsense").unwrap_err().to_string();
        assert!(e.contains("unknown backend"), "{e}");
    }

    #[test]
    fn custom_backends() {
        let env = Env::for_test(&[("MY_KEY", "k")], vec![], &[], &[]);
        let mut cfg = Config::default();
        let d = detect_with_locals(&cfg, &env, no_locals());
        assert!(d.found.is_empty());
        let e = d.pick("custom").unwrap_err().to_string();
        assert!(e.contains("not configured"), "{e}");

        cfg.provider.custom.base_url = "http://localhost:9000/v1".into();
        cfg.provider.custom.model = "m".into();
        cfg.provider.custom.api_key_env = "MY_KEY".into();
        let d = detect_with_locals(&cfg, &env, no_locals());
        let b = d.pick("auto").unwrap();
        assert_eq!((b.id.as_str(), b.model.as_str()), ("custom", "m"));
        match &b.transport {
            Transport::OpenAi(w) => assert!(w.local && w.api_key.is_some()),
            t => panic!("unexpected {t:?}"),
        }

        cfg.provider.custom.kind = ProviderKind::Anthropic;
        cfg.provider.custom.api_key_env = "UNSET".into();
        let d = detect_with_locals(&cfg, &env, no_locals());
        assert!(d.missing.iter().any(|m| m.id == "custom"));

        cfg.provider.custom.kind = ProviderKind::Command;
        cfg.provider.custom.command = vec![which::which("cat").unwrap().display().to_string()];
        let d = detect_with_locals(&cfg, &env, no_locals());
        assert_eq!(d.pick("custom").unwrap().id, "custom");
    }

    #[test]
    fn model_lists_skip_embeddings() {
        let tags =
            serde_json::json!({"models":[{"name":"nomic-embed-text:latest"},{"name":"qwen3:8b"}]});
        assert_eq!(parse_model_list(ModelList::OllamaTags, &tags), ["qwen3:8b"]);
        let oai = serde_json::json!({"data":[{"id":"a"},{"id":"b"}]});
        assert_eq!(parse_model_list(ModelList::OpenAi, &oai), ["a", "b"]);
    }

    #[test]
    fn ollama_host_variants() {
        let l = &LOCALS[0];
        for (v, want) in [
            ("0.0.0.0", "http://127.0.0.1:11434"),
            ("0.0.0.0:11500", "http://127.0.0.1:11500"),
            ("http://box.lan:11434/", "http://box.lan:11434"),
        ] {
            let env = Env::for_test(&[("OLLAMA_HOST", v)], vec![], &[], &[]);
            assert_eq!(local_bases(l, &env), [want], "{v}");
        }
    }

    #[test]
    fn local_url_detection() {
        assert!(is_local_url("http://localhost:8080/v1"));
        assert!(is_local_url("http://127.0.0.1:1/v1"));
        assert!(!is_local_url("https://api.deepseek.com"));
    }

    #[test]
    fn catalog_ids_are_unique_and_in_default_order() {
        let ids = known_ids();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        for id in &ids {
            assert!(
                PINNED_ONLY.contains(id) || crate::config::DEFAULT_ORDER.contains(id),
                "{id} missing from DEFAULT_ORDER"
            );
        }
        for api in APIS {
            if let Wire::OpenAi { extra, .. } = api.wire {
                assert!(serde_json::from_str::<Value>(extra).unwrap().is_object());
            }
        }
    }
}

#[cfg(test)]
#[path = "backends_qa_tests.rs"]
mod qa_tests;
