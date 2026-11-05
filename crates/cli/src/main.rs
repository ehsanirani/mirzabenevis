use anyhow::Result;
use clap::{Parser, Subcommand};
use mirzabenevis_core::{text, Config};
use std::io::{IsTerminal, Read, Write};

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
    },
    /// List configured actions
    Actions,
    /// Show the current selection (useful for testing grab commands)
    Grab,
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
    /// Set the model, e.g. `config model llama3.1:8b`
    Model { name: String },
    /// Set provider kind and base URL, e.g. `config provider openai http://localhost:11434/v1`
    Provider { kind: String, base_url: Option<String> },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_env("MIRZABENEVIS_LOG"),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let mut cfg = Config::load()?;

    match cli.cmd {
        Cmd::Actions => {
            for a in &cfg.actions {
                println!("{:<12} {:<3} {}", a.id, a.key, a.name);
            }
        }
        Cmd::Grab => {
            print!("{}", text::grab(&cfg)?);
        }
        Cmd::Config { cmd } => match cmd {
            ConfigCmd::Path => println!("{}", Config::path()?.display()),
            ConfigCmd::Init => println!("wrote {}", Config::init()?.display()),
            ConfigCmd::Show => print!("{}", toml::to_string_pretty(&cfg)?),
            ConfigCmd::Model { name } => {
                cfg.provider.model = name;
                println!("saved {}", cfg.save()?.display());
            }
            ConfigCmd::Provider { kind, base_url } => {
                cfg.provider.kind = match kind.as_str() {
                    "openai" => mirzabenevis_core::config::ProviderKind::OpenAi,
                    "anthropic" => mirzabenevis_core::config::ProviderKind::Anthropic,
                    other => anyhow::bail!("unknown provider kind `{other}` (openai | anthropic)"),
                };
                if let Some(u) = base_url {
                    cfg.provider.base_url = u;
                }
                println!("saved {}", cfg.save()?.display());
            }
        },
        Cmd::Run { action, print, copy, stdin, prompt } => {
            // Input: --stdin, or piped stdin, otherwise the current selection.
            let input = if stdin || !std::io::stdin().is_terminal() {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                text::grab(&cfg)?
            };

            // One-off prompt: inject a temporary action.
            let action_id = if let Some(p) = prompt {
                cfg.actions.push(mirzabenevis_core::Action {
                    id: "_adhoc".into(),
                    name: "ad hoc".into(),
                    key: String::new(),
                    prompt: p,
                });
                "_adhoc".to_string()
            } else {
                action.unwrap_or_else(|| cfg.general.default_action.clone())
            };

            let stream_to_stdout = print && std::io::stdout().is_terminal();
            let result = mirzabenevis_core::run_action(&cfg, &action_id, &input, |chunk| {
                if stream_to_stdout {
                    print!("{chunk}");
                    let _ = std::io::stdout().flush();
                }
            })
            .await?;

            if print {
                if stream_to_stdout {
                    println!();
                } else {
                    println!("{result}");
                }
            } else if copy || !cfg.general.auto_paste {
                text::set_clipboard(&result)?;
                eprintln!("copied to clipboard");
            } else {
                match text::paste(&cfg, &result)? {
                    text::PasteResult::Typed => {}
                    text::PasteResult::Clipboard => eprintln!("no paste tool found; copied to clipboard"),
                }
            }
        }
    }
    Ok(())
}
