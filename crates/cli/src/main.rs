use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use mirzabenevis_core::config::Output;
use mirzabenevis_core::{backends, history, provider, text, Backend, Config, Detection};
use std::io::{IsTerminal, Read, Write};
use std::time::{Duration, Instant};

mod doctor;

/// mirzabenevis: select text anywhere, run an action, get the result pasted back.
///
/// Bind e.g. `mirzabenevis-cli run fix` to a key in your compositor.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run an action on the current selection (or stdin) and paste the result
    /// (or copy/print it, following the action's `output`)
    Run {
        /// Action id from config (default: general.default_action)
        action: Option<String>,
        /// Print the result to stdout instead of pasting it
        #[arg(long, short)]
        print: bool,
        /// Copy the result to the clipboard instead of typing it
        #[arg(long, short)]
        copy: bool,
        /// Read input from stdin instead of the selection
        #[arg(long)]
        stdin: bool,
        /// Run a one-off prompt instead of a configured action ({text} optional)
        #[arg(long)]
        prompt: Option<String>,
        /// Use this backend instead of provider.use (see `providers`)
        #[arg(long, short)]
        backend: Option<String>,
        /// Language for {target} in the prompt (default: translate.target), e.g. `--target de`
        #[arg(long)]
        target: Option<String>,
    },
    /// Show the last runs (original and result)
    History {
        /// How many
        #[arg(short, default_value_t = 10)]
        n: usize,
    },
    /// Paste the original text of the last run that replaced a selection
    Undo,
    /// List the AI backends found on this machine (API keys, agent CLIs, local servers)
    Providers {
        /// Send a tiny request to every backend found and report how it went
        #[arg(long)]
        check: bool,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
    /// List configured actions
    Actions,
    /// Show the current selection (useful for testing grab commands)
    Grab,
    /// Check this desktop: grab/paste tools, AI backends, and how to bind the key
    Doctor,
    /// Manage the config file
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the config file path
    Path,
    /// Write the default config file (does not overwrite)
    Init,
    /// Print the effective config
    Show,
    /// Choose the backend: `auto` or an id from `providers`, e.g. `config use claude-code`
    Use { backend: String },
    /// Set the model of a backend (default: the one in use), e.g. `config model deepseek-v4-pro`
    Model {
        name: String,
        #[arg(long, short)]
        backend: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("MIRZABENEVIS_LOG"))
        .with_writer(std::io::stderr)
        .init();

    // Behave like other CLIs when piped into `head`: exit quietly, no panic.
    // SAFETY: setting a signal disposition at startup, before any threads use it.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    // These work even when the config file is broken.
    if let Cmd::Config { cmd } = &cli.cmd {
        match cmd {
            ConfigCmd::Path => {
                println!("{}", Config::path()?.display());
                return Ok(());
            }
            ConfigCmd::Init => {
                println!("wrote {}", Config::init()?.display());
                return Ok(());
            }
            _ => {}
        }
    }
    let mut cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            if matches!(cli.cmd, Cmd::Run { .. } | Cmd::Undo) {
                notify_failure(&Config::default(), &e);
            }
            return Err(e);
        }
    };

    match cli.cmd {
        Cmd::Actions => {
            for a in &cfg.actions {
                println!("{:<12} {:<3} {}", a.id, a.key, a.name);
            }
        }
        Cmd::Grab => {
            print!("{}", text::grab(&cfg)?);
        }
        Cmd::Doctor => {
            if !doctor::run(&cfg).await {
                std::process::exit(1);
            }
        }
        Cmd::History { n } => {
            // Replacement records repeat a run; they are only there for `undo`.
            let h: Vec<_> = history::read()?
                .into_iter()
                .filter(|e| !e.replaced)
                .collect();
            let now = now_secs();
            for e in &h[h.len().saturating_sub(n)..] {
                let what = e.prompt.as_deref().unwrap_or(&e.action);
                println!(
                    "{} ago · {} · {}",
                    age(now.saturating_sub(e.ts)),
                    what,
                    e.backend
                );
                println!("  - {}", one_line(&e.original));
                println!("  + {}", one_line(&e.result));
            }
        }
        Cmd::Undo => {
            let h = history::read()?;
            let Some(e) = history::last_undoable(&h) else {
                bail!("nothing to undo");
            };
            let how = deliver(false, false, cfg.general.auto_paste, Output::Replace);
            let r = deliver_result(&cfg, &e.original, how).map(drop);
            if let Err(e) = &r {
                notify_failure(&cfg, e);
            }
            return r;
        }
        Cmd::Providers { check, json } => {
            // Always re-read the login shell here, so a key just added to
            // ~/.zshrc shows up.
            let det = backends::detect_current(&cfg, true).await;
            let checks = if check {
                if !json {
                    print_providers(&cfg, &det);
                    println!("\nChecking each backend with a tiny request…");
                }
                check_backends(&cfg, &det, !json).await
            } else {
                Vec::new()
            };
            if json {
                let out = serde_json::json!({ "detection": det, "checks": checks });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if !check {
                print_providers(&cfg, &det);
            }
        }
        Cmd::Config { cmd } => match cmd {
            ConfigCmd::Path | ConfigCmd::Init => unreachable!("handled above"),
            ConfigCmd::Show => {
                if !cfg.provider.custom.api_key.is_empty() {
                    cfg.provider.custom.api_key = "<hidden>".into();
                }
                print!("{}", toml::to_string_pretty(&cfg)?)
            }
            ConfigCmd::Use { backend } => {
                let id = backend.trim();
                if id != "auto" && !backends::known_ids().contains(&id) {
                    bail!(
                        "unknown backend `{id}`; choose auto or one of: {}",
                        backends::known_ids().join(", ")
                    );
                }
                let path = Config::set_in_file(&["provider", "use"], id)?;
                println!("saved {}", path.display());
                cfg.provider.use_backend = id.to_string();
                let det = backends::detect_current(&cfg, false).await;
                match det.pick(id) {
                    Ok(b) => println!("using {} ({})", b.label, b.model_label()),
                    Err(e) => eprintln!("warning: {e:#}"),
                }
            }
            ConfigCmd::Model { name, backend } => {
                let pinned = cfg.provider.use_backend.trim();
                let id = match backend {
                    Some(id) => id,
                    None if pinned != "auto" && !pinned.is_empty() => pinned.to_string(),
                    None => {
                        let det = backends::detect_current(&cfg, false).await;
                        det.pick("auto")?.id.clone()
                    }
                };
                if !backends::known_ids().contains(&id.as_str()) {
                    bail!("unknown backend `{id}` (see `mirzabenevis-cli providers`)");
                }
                let path = Config::set_in_file(&["provider", "models", &id], &name)?;
                println!("{id} now uses {name} (saved {})", path.display());
            }
        },
        Cmd::Run {
            action,
            print,
            copy,
            stdin,
            prompt,
            backend,
            target,
        } => {
            if let Some(t) = target {
                cfg.translate.target = t;
            }
            let r = run(&mut cfg, action, print, copy, stdin, prompt, backend).await;
            if let Err(e) = &r {
                notify_failure(&cfg, e);
            }
            return r;
        }
    }
    Ok(())
}

/// A desktop notification when nobody sees stderr (started from a keybind).
/// Best effort: missing `notify-send` is fine.
fn notify(cfg: &Config, body: &str) {
    if cfg.general.notify && !std::io::stderr().is_terminal() {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "mirzabenevis", "mirzabenevis", body])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

fn notify_failure(cfg: &Config, e: &anyhow::Error) {
    let msg = format!("{e:#}");
    notify(
        cfg,
        &format!("Failed: {}", msg.lines().next().unwrap_or("")),
    );
}

async fn run(
    cfg: &mut Config,
    action: Option<String>,
    print: bool,
    copy: bool,
    stdin: bool,
    prompt: Option<String>,
    backend: Option<String>,
) -> Result<()> {
    // Input: --stdin, or piped stdin, otherwise the current selection.
    // A compositor keybind starts us with a non-terminal but empty stdin
    // (usually /dev/null), so empty stdin falls back to the selection.
    let mut input = String::new();
    if stdin || !std::io::stdin().is_terminal() {
        std::io::stdin().read_to_string(&mut input)?;
    }
    if input.trim().is_empty() {
        if stdin {
            bail!("--stdin given but stdin was empty");
        }
        input = text::grab(cfg)?;
        if input.trim().is_empty() {
            eprintln!("nothing selected");
            notify(cfg, "Nothing selected");
            std::process::exit(1);
        }
    }

    // One-off prompt: inject a temporary action.
    let action_id = if let Some(p) = prompt {
        cfg.actions.push(mirzabenevis_core::Action {
            id: "_adhoc".into(),
            name: "ad hoc".into(),
            prompt: p,
            ..Default::default()
        });
        "_adhoc".to_string()
    } else {
        action.unwrap_or_else(|| cfg.general.default_action.clone())
    };
    let Some(act) = cfg.action(&action_id) else {
        bail!("unknown action `{action_id}` (see `mirzabenevis-cli actions`)");
    };
    let how = deliver(print, copy, cfg.general.auto_paste, act.output);

    let det = backends::detect_current(cfg, false).await;
    // Same precedence as run_action_auto: the action's own backend wins.
    let wanted = act
        .backend
        .clone()
        .or(backend)
        .unwrap_or_else(|| cfg.provider.use_backend.clone());
    let first = det.pick(&wanted)?;
    let tty = std::io::stderr().is_terminal();
    if tty {
        eprintln!("\x1b[2m{} · {}\x1b[0m", first.label, first.model_label());
    }

    let stream_to_stdout = how == Deliver::Print && std::io::stdout().is_terminal();
    let (result, used) =
        mirzabenevis_core::run_action_auto(cfg, &det, &wanted, &action_id, &input, |chunk| {
            if stream_to_stdout {
                print!("{chunk}");
                let _ = std::io::stdout().flush();
            }
        })
        .await?;
    tracing::info!(
        "answered by {} ({}) via {}",
        used.label,
        used.model_label(),
        used.source
    );
    if used.id != first.id {
        if tty {
            eprintln!(
                "\x1b[2m(answered by {} · {})\x1b[0m",
                used.label,
                used.model_label()
            );
        }
        notify(cfg, &format!("Answered by {} (fallback)", used.label));
    }

    if how == Deliver::Print {
        if stream_to_stdout {
            println!();
        } else if result.ends_with('\n') {
            print!("{result}");
        } else {
            println!("{result}");
        }
        // Started from a keybind, stdout goes nowhere: show it.
        if !print {
            notify(cfg, &result);
        }
        return Ok(());
    }
    if deliver_result(cfg, &result, how)? {
        let entry = history::Entry {
            replaced: true,
            ..history::Entry::new(&action_id, None, &used.id, &input, &result)
        };
        history::append(cfg, &entry);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Deliver {
    Print,
    Copy,
    Paste,
}

/// Flags win over the action's `output`; `auto_paste = false` turns pasting into copying.
fn deliver(print: bool, copy: bool, auto_paste: bool, output: Output) -> Deliver {
    match output {
        _ if print => Deliver::Print,
        _ if copy => Deliver::Copy,
        Output::Show => Deliver::Print,
        Output::Clipboard => Deliver::Copy,
        Output::Replace if copy || !auto_paste => Deliver::Copy,
        Output::Replace => Deliver::Paste,
    }
}

/// Copy or paste `result`; true when it was pasted over the selection.
fn deliver_result(cfg: &Config, result: &str, how: Deliver) -> Result<bool> {
    if how == Deliver::Copy {
        text::set_clipboard(result)?;
        eprintln!("copied to clipboard");
        return Ok(false);
    }
    Ok(match text::paste(cfg, result)? {
        // Ctrl+V can silently do nothing (e.g. in a terminal): keep a copy.
        text::PasteResult::Pasted => {
            eprintln!("{result}");
            true
        }
        text::PasteResult::Typed => true,
        text::PasteResult::Clipboard => {
            eprintln!("no paste tool found; copied to clipboard");
            false
        }
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn age(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

/// First line, at most 80 characters, marked when cut.
fn one_line(s: &str) -> String {
    let first = s.trim().lines().next().unwrap_or("");
    let cut: String = first.chars().take(80).collect();
    if cut.len() < s.trim().len() {
        format!("{cut}…")
    } else {
        cut
    }
}

fn print_providers(cfg: &Config, det: &Detection) {
    let chosen = det
        .pick(&cfg.provider.use_backend)
        .ok()
        .map(|b| b.id.clone());
    if det.found.is_empty() {
        println!("No AI backend found.\n");
    } else {
        println!("Found (in auto order):");
        for b in &det.found {
            let mark = if Some(&b.id) == chosen.as_ref() {
                "▶"
            } else {
                " "
            };
            println!(
                "{mark} {:<12} {:<18} {:<26} {}",
                b.id,
                b.label,
                b.model_label(),
                b.source
            );
        }
        println!();
    }
    println!("Not available:");
    for m in &det.missing {
        println!("  {:<12} {:<18} {}", m.id, m.label, m.hint);
    }
    println!();
    match det.pick(&cfg.provider.use_backend) {
        Ok(b) => println!(
            "Using {} (provider.use = \"{}\"). Change it with `mirzabenevis-cli config use <id>`.",
            b.id, cfg.provider.use_backend
        ),
        Err(e) => println!("Not usable: {e:#}"),
    }
    if det.used_login_shell {
        println!("(Also looked at your login shell's environment.)");
    }
}

#[derive(serde::Serialize)]
struct Check {
    id: String,
    ok: bool,
    seconds: f32,
    reply: String,
}

async fn check_backends(cfg: &Config, det: &Detection, print: bool) -> Vec<Check> {
    let mut gen = provider::Gen::from(&cfg.provider);
    gen.max_tokens = gen.max_tokens.min(256);
    gen.timeout = gen.timeout.min(Duration::from_secs(60));
    let checks = det.found.iter().map(|b: &Backend| {
        let gen = gen.clone();
        async move {
            let t = Instant::now();
            let r = provider::complete(
                b,
                &gen,
                "Reply with exactly the word OK and nothing else.",
                "Say OK.",
            )
            .await;
            (b, t.elapsed(), r)
        }
    });
    let mut out = Vec::new();
    for (b, took, r) in futures_util::future::join_all(checks).await {
        let (ok, reply) = match r {
            Ok(text) => (true, text.trim().chars().take(40).collect::<String>()),
            Err(e) => (false, format!("{e:#}").chars().take(300).collect()),
        };
        if print {
            let mark = if ok { "✓" } else { "✗" };
            let reply = if ok {
                format!("{reply:?}")
            } else {
                reply.clone()
            };
            println!(
                "  {mark} {:<12} {:>5.1}s  {reply}",
                b.id,
                took.as_secs_f32()
            );
        }
        out.push(Check {
            id: b.id.clone(),
            ok,
            seconds: took.as_secs_f32(),
            reply,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_mode_and_flags() {
        use Deliver::*;
        use Output::*;
        let d = |print, copy, auto, out| deliver(print, copy, auto, out);
        assert_eq!(d(false, false, true, Replace), Paste);
        assert_eq!(d(false, true, true, Replace), Copy);
        assert_eq!(d(false, false, false, Replace), Copy);
        assert_eq!(d(false, false, true, Clipboard), Copy);
        assert_eq!(d(false, true, true, Show), Copy);
        assert_eq!(d(false, false, false, Show), Print);
        assert_eq!(d(true, false, true, Clipboard), Print);
    }
}
