//! Getting text out of, and back into, whatever app the user is in.
//!
//! Linux strategy (first priority):
//! - grab: read the *primary selection* with `wl-paste --primary` or `xclip`,
//!   so highlighting text is enough; no Ctrl+C is simulated. The clipboard is
//!   used only with `grab = "clipboard"` or `grab_fallback_clipboard = true`.
//! - paste: put the result on the clipboard and press Ctrl+V (`wtype`,
//!   `ydotool`, `xdotool`), then restore the old clipboard; or type it in with
//!   `paste_cmds`. If nothing works, leave it on the clipboard.
//!
//! Every command is configurable in `[linux]` so it works across wlroots,
//! KDE, GNOME (with ydotool) and X11 without code changes.

use crate::config::{Config, GrabSource, PasteMode};
use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Where the text ended up after `paste`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteResult {
    /// Clipboard + simulated Ctrl+V.
    Pasted,
    /// Typed keystroke by keystroke.
    Typed,
    /// Only reached the clipboard.
    Clipboard,
}

/// What `paste` would do right now, judged by which tools exist in PATH.
/// Lets a caller decide (e.g. keep a window open) before pasting.
pub fn paste_method(cfg: &Config) -> PasteResult {
    if cfg.linux.paste == PasteMode::Clipboard
        && first_available(&cfg.linux.paste_key_cmds).is_some()
    {
        PasteResult::Pasted
    } else if first_available(&cfg.linux.paste_cmds).is_some() {
        PasteResult::Typed
    } else {
        PasteResult::Clipboard
    }
}

/// The selection, or `""` when nothing is selected. `Err` only when no tool
/// can be run at all.
pub fn grab(cfg: &Config) -> Result<String> {
    if cfg.linux.grab == GrabSource::Primary {
        let s = if first_available(&cfg.linux.grab_cmds).is_some() {
            // wl-paste exits non-zero when nothing is selected: that is just empty.
            run_until_ok(&cfg.linux.grab_cmds, None)
                .map(|(_, out)| String::from_utf8_lossy(&out).into_owned())
                .unwrap_or_default()
        } else {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
            match cb.get().clipboard(LinuxClipboardKind::Primary).text() {
                Err(arboard::Error::ContentNotAvailable) => String::new(),
                r => r.context("reading the primary selection")?,
            }
        };
        if !s.trim().is_empty() || !cfg.linux.grab_fallback_clipboard {
            return Ok(s);
        }
        tracing::debug!("primary selection empty, falling back to clipboard");
    }
    clipboard_text()
}

/// Put `text` where the cursor is. See [`paste_method`] for the order tried.
pub fn paste(cfg: &Config, text: &str) -> Result<PasteResult> {
    if paste_method(cfg) == PasteResult::Pasted {
        let old = saved_clipboard();
        // Without wl-copy this is the arboard owner: it must live until the
        // app has read the clipboard and the old text is back.
        let mut owner = hold_clipboard(text)?;
        let pressed = run_until_ok(&cfg.linux.paste_key_cmds, None).is_some();
        if pressed {
            // Let the app read the clipboard before we put the old text back.
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        if let Some(old) = old {
            let _ = match owner.as_mut() {
                Some(cb) => cb.set_text(old).map_err(Into::into),
                None => set_clipboard(&old),
            };
        }
        if pressed {
            return Ok(PasteResult::Pasted);
        }
    }
    if run_until_ok(&cfg.linux.paste_cmds, Some(text)).is_some() {
        return Ok(PasteResult::Typed);
    }
    if cfg.linux.fallback_to_clipboard {
        set_clipboard(text)?;
        return Ok(PasteResult::Clipboard);
    }
    Err(anyhow!(
        "no paste command worked (see [linux].paste_key_cmds and paste_cmds)"
    ))
}

/// Whether `prog` is in PATH.
pub fn on_path(prog: &str) -> bool {
    which::which(prog).is_ok()
}

/// Whether the session is Wayland. `wl-*` tools are skipped outside it and
/// `xdotool` inside it (it would only reach XWayland windows).
pub fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn fits_session(cmd: &[String], wayland: bool) -> bool {
    let Some(prog) = cmd.first() else {
        return false;
    };
    let name = prog.rsplit('/').next().unwrap_or(prog);
    if wayland {
        name != "xdotool"
    } else {
        !name.starts_with("wl-")
    }
}

/// Run the commands found in PATH (and fitting the session) in order until one
/// exits successfully, with `input` on stdin. Returns it and its stdout.
fn run_until_ok<'a>(
    cmds: &'a [Vec<String>],
    input: Option<&str>,
) -> Option<(&'a Vec<String>, Vec<u8>)> {
    let wayland = wayland();
    cmds.iter()
        .filter(|c| fits_session(c, wayland) && which::which(&c[0]).is_ok())
        .find_map(|cmd| match run(cmd, input) {
            Ok(out) => Some((cmd, out)),
            Err(e) => {
                tracing::warn!("{} failed: {e:#}", cmd.join(" "));
                None
            }
        })
}

fn run(cmd: &[String], input: Option<&str>) -> Result<Vec<u8>> {
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let written = match (input, child.stdin.take()) {
        (Some(text), Some(mut stdin)) => stdin.write_all(text.as_bytes()),
        _ => Ok(()),
    };
    // Reap the child even if it stopped reading early (EPIPE).
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "{}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    written.context("writing to stdin")?;
    Ok(out.stdout)
}

/// Current clipboard text, `""` if it is empty or holds no text.
fn clipboard_text() -> Result<String> {
    let wl = [vec![
        "wl-paste".into(),
        "--no-newline".into(),
        "--type".into(),
        "text".into(),
    ]];
    if wayland() && first_available(&wl).is_some() {
        // Non-zero: nothing (or no text) on the clipboard.
        return Ok(run_until_ok(&wl, None)
            .and_then(|(_, out)| String::from_utf8(out).ok())
            .unwrap_or_default());
    }
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    match cb.get_text() {
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        r => r.context("reading clipboard"),
    }
}

/// The clipboard text to put back after pasting; `None` when empty or marked
/// as a password by a password manager (we do not keep or re-publish those).
fn saved_clipboard() -> Option<String> {
    if wayland() {
        let types = Command::new("wl-paste").arg("--list-types").output();
        if let Ok(t) = types {
            if String::from_utf8_lossy(&t.stdout).contains("x-kde-passwordManagerHint") {
                return None;
            }
        }
    }
    clipboard_text().ok().filter(|s| !s.is_empty())
}

/// On Wayland the clipboard content dies with the process that set it, so
/// `wl-copy` (which stays resident) is used there when present. Otherwise
/// `arboard` is used and the text lasts only while this process runs (unless
/// a clipboard manager takes it over).
pub fn set_clipboard(text: &str) -> Result<()> {
    hold_clipboard(text).map(drop)
}

/// [`set_clipboard`], returning the arboard owner (if it was used) so the
/// caller can keep the content alive.
fn hold_clipboard(text: &str) -> Result<Option<arboard::Clipboard>> {
    if wayland() && which::which("wl-copy").is_ok() {
        // wl-copy forks a server and its parent exits: wait for the parent (no
        // zombie), with no pipes the server could hold open.
        let mut child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("running wl-copy")?;
        let written = child
            .stdin
            .take()
            .expect("stdin piped")
            .write_all(text.as_bytes());
        if child.wait()?.success() && written.is_ok() {
            return Ok(None);
        }
        tracing::warn!("wl-copy failed, using arboard");
    }
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    cb.set_text(text.to_string()).context("writing clipboard")?;
    Ok(Some(cb))
}

/// The first command that is in PATH and fits the session (Wayland or X11).
pub fn first_available(cmds: &[Vec<String>]) -> Option<&Vec<String>> {
    let wayland = wayland();
    cmds.iter()
        .find(|c| fits_session(c, wayland) && which::which(&c[0]).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_tried_until_one_succeeds() {
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cmds = vec![
            c(&["no-such-tool-xyz"]),
            c(&["false"]),
            c(&["cat"]),
            c(&["true"]),
        ];
        let (cmd, out) = run_until_ok(&cmds, Some("hi")).unwrap();
        assert_eq!((cmd, out.as_slice()), (&cmds[2], b"hi".as_slice()));
        assert!(run_until_ok(&cmds[..2], None).is_none());
        // A child that exits without reading its stdin is still reaped and reported.
        let big = "x".repeat(1 << 20);
        assert!(run(&c(&["true"]), Some(&big)).is_err());
    }

    #[test]
    fn tools_fit_the_session() {
        let c = |s: &str| vec![s.to_string()];
        assert!(!fits_session(&c("xdotool"), true));
        assert!(fits_session(&c("/usr/bin/xdotool"), false));
        assert!(!fits_session(&c("wl-paste"), false));
        assert!(fits_session(&c("wl-paste"), true));
        assert!(fits_session(&c("ydotool"), true) && fits_session(&c("ydotool"), false));
    }
}
