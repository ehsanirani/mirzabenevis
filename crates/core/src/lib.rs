//! mirzabenevis core: everything that does not depend on a UI.
//!
//! - [`config`]   load/save the TOML config and the list of actions
//! - [`provider`] stream completions from OpenAI-compatible or Anthropic APIs
//! - [`text`]     grab the selected text and paste a result back (Linux first)
//! - [`run_action`] glue the three together

pub mod config;
pub mod provider;
pub mod text;

pub use config::{Action, Config};

use anyhow::{Context, Result};

/// Run one action over `input`, streaming chunks to `on_chunk`, and return the full result.
pub async fn run_action(
    cfg: &Config,
    action_id: &str,
    input: &str,
    on_chunk: impl FnMut(&str),
) -> Result<String> {
    let action = cfg
        .action(action_id)
        .with_context(|| format!("unknown action `{action_id}` (see `config actions`)"))?;
    let user = action.render(input);
    let out = provider::stream(&cfg.provider, &cfg.system_prompt, &user, on_chunk).await?;
    Ok(clean_output(&out))
}

/// Models love to wrap answers in fences or quotes despite being told not to.
pub fn clean_output(s: &str) -> String {
    let mut t = s.trim();
    if t.starts_with("```") {
        if let Some(nl) = t.find('\n') {
            t = &t[nl + 1..];
        }
        t = t.strip_suffix("```").unwrap_or(t).trim_end();
    }
    if t.len() > 2 && t.starts_with('"') && t.ends_with('"') && !t[1..t.len() - 1].contains('"') {
        t = &t[1..t.len() - 1];
    }
    t.to_string()
}
