//! Getting text out of, and back into, whatever app the user is in.
//!
//! Linux strategy (first priority):
//! - grab: read the *primary selection* with `wl-paste --primary` or `xclip`,
//!   so highlighting text is enough; no Ctrl+C is simulated. Falls back to the
//!   clipboard via `arboard` if no command is available or `grab = "clipboard"`.
//! - paste: pipe the result into `wtype` / `xdotool type` (or any command the
//!   user configures). If none exists, put it on the clipboard instead.
//!
//! Every command is configurable in `[linux]` so it works across wlroots,
//! KDE, GNOME (with ydotool) and X11 without code changes.

use crate::config::{Config, GrabSource};
use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Where the text ended up after `paste`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteResult {
    Typed,
    Clipboard,
}

pub fn grab(cfg: &Config) -> Result<String> {
    if cfg.linux.grab == GrabSource::Primary {
        if let Some(cmd) = first_available(&cfg.linux.grab_cmds) {
            let out = Command::new(&cmd[0])
                .args(&cmd[1..])
                .stderr(Stdio::null())
                .output()
                .with_context(|| format!("running {}", cmd.join(" ")))?;
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            if !s.trim().is_empty() {
                return Ok(s);
            }
            tracing::debug!("primary selection empty, falling back to clipboard");
        }
    }
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    let s = cb.get_text().context("reading clipboard")?;
    if s.trim().is_empty() {
        return Err(anyhow!("nothing selected and clipboard is empty"));
    }
    Ok(s)
}

pub fn paste(cfg: &Config, text: &str) -> Result<PasteResult> {
    if let Some(cmd) = first_available(&cfg.linux.paste_cmds) {
        let mut child = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("running {}", cmd.join(" ")))?;
        child
            .stdin
            .take()
            .expect("stdin piped")
            .write_all(text.as_bytes())?;
        let out = child.wait_with_output()?;
        if out.status.success() {
            return Ok(PasteResult::Typed);
        }
        tracing::warn!(
            "{} failed: {}",
            cmd.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    if cfg.linux.fallback_to_clipboard {
        set_clipboard(text)?;
        return Ok(PasteResult::Clipboard);
    }
    Err(anyhow!("no paste command available (see [linux].paste_cmds)"))
}

pub fn set_clipboard(text: &str) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    cb.set_text(text.to_string()).context("writing clipboard")?;
    // On Wayland the clipboard content dies with the process that set it.
    // wl-copy stays resident, so prefer it when present.
    if let Ok(path) = which::which("wl-copy") {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        child.stdin.take().expect("stdin").write_all(text.as_bytes())?;
        // Intentionally not waited on: wl-copy forks and serves the clipboard.
    }
    Ok(())
}

fn first_available(cmds: &[Vec<String>]) -> Option<&Vec<String>> {
    cmds.iter()
        .filter(|c| !c.is_empty())
        .find(|c| which::which(&c[0]).is_ok())
}
