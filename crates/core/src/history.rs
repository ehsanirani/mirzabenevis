//! Successful runs, newest last, in `$XDG_DATA_HOME/mirzabenevis/history.jsonl`
//! (one JSON object per line; trimmed to the newest [`CAP`] once it holds twice
//! that). Feeds `undo` and the popup's recent instructions. Plain text, private
//! to the user (mode 600); off with `general.history = false`.

use crate::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CAP: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix seconds.
    pub ts: u64,
    pub action: String,
    /// The instruction of a one-off prompt (action ids starting with `_`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Backend id.
    pub backend: String,
    pub original: String,
    pub result: String,
    /// The result was pasted over `original` (what `undo` reverts). Runs are
    /// logged when they finish, replacements again when they are pasted.
    #[serde(default)]
    pub replaced: bool,
}

impl Entry {
    pub fn new(
        action: &str,
        prompt: Option<String>,
        backend: &str,
        original: &str,
        result: &str,
    ) -> Self {
        Entry {
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            action: action.into(),
            prompt,
            backend: backend.into(),
            original: original.into(),
            result: result.into(),
            replaced: false,
        }
    }
}

pub fn path() -> Result<PathBuf> {
    let dirs =
        directories::ProjectDirs::from("", "", "mirzabenevis").context("no home directory")?;
    Ok(dirs.data_dir().join("history.jsonl"))
}

/// All entries, oldest first. No file = no history; unreadable lines are skipped.
pub fn read() -> Result<Vec<Entry>> {
    read_from(&path()?)
}

/// Add an entry unless `general.history` is off. Failing to write only logs.
pub fn append(cfg: &Config, entry: &Entry) {
    if !cfg.general.history {
        return;
    }
    if let Err(e) = path().and_then(|p| append_to(&p, entry, CAP)) {
        tracing::warn!("history not saved: {e:#}");
    }
}

/// The newest replacement that was actually pasted: the one `undo` reverts.
pub fn last_undoable(entries: &[Entry]) -> Option<&Entry> {
    entries.iter().rev().find(|e| e.replaced)
}

/// The last `n` distinct one-off instructions, newest first (case-insensitive;
/// the newest spelling wins).
pub fn recent_prompts(entries: &[Entry], n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in entries.iter().rev().filter_map(|e| e.prompt.as_ref()) {
        if out.len() == n {
            break;
        }
        if !out.iter().any(|o| o.to_lowercase() == p.to_lowercase()) {
            out.push(p.clone());
        }
    }
    out
}

fn read_from(p: &Path) -> Result<Vec<Entry>> {
    match std::fs::read_to_string(p) {
        Ok(s) => Ok(s
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
    }
}

fn open(p: &Path, append: bool) -> Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.create(true).append(append).write(true).truncate(!append);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(p)
        .with_context(|| format!("writing {}", p.display()))
}

/// Append one line (a single write, so concurrent appends do not interleave).
/// Past `2 * cap` lines, rewrite the file with the newest `cap`.
// ponytail: the trim reads, then renames: an append from another process in
// between is lost. Rare (one trim per `cap` runs); add a lock file if it matters.
fn append_to(p: &Path, entry: &Entry, cap: usize) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let line = format!("{}\n", serde_json::to_string(entry)?); // JSON escapes newlines
    open(p, true)?.write_all(line.as_bytes())?;
    let all = std::fs::read_to_string(p)?;
    let n = all.lines().count();
    if n <= 2 * cap {
        return Ok(());
    }
    let keep: String = all.lines().skip(n - cap).flat_map(|l| [l, "\n"]).collect();
    let tmp = p.with_extension(format!("jsonl.{}.tmp", std::process::id()));
    open(&tmp, false)?.write_all(keep.as_bytes())?;
    std::fs::rename(&tmp, p).with_context(|| format!("writing {}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(i: u64, action: &str, prompt: Option<&str>) -> Entry {
        Entry {
            ts: i,
            prompt: prompt.map(Into::into),
            original: format!("o{i}\nline"),
            ..Entry::new(action, None, "b", "", &format!("r{i}"))
        }
    }

    #[test]
    fn capped_and_rotated() {
        let dir = std::env::temp_dir().join(format!("mzb-history-{}", std::process::id()));
        let p = dir.join("h.jsonl");
        let _ = std::fs::remove_file(&p);
        let ts = |p: &Path| -> Vec<u64> { read_from(p).unwrap().iter().map(|e| e.ts).collect() };
        for i in 0..6 {
            append_to(&p, &entry(i, "fix", None), 3).unwrap();
        }
        assert_eq!(ts(&p), [0, 1, 2, 3, 4, 5]); // up to 2 * cap kept
        append_to(&p, &entry(6, "fix", None), 3).unwrap();
        assert_eq!(ts(&p), [4, 5, 6]);
        assert_eq!(read_from(&p).unwrap()[0].original, "o4\nline");
        assert!(read_from(&dir.join("none")).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_and_recents() {
        let mut h = vec![
            entry(0, "fix", None),
            entry(1, "_custom", Some("a")),
            entry(2, "_custom", Some("b")),
            entry(3, "_adhoc", Some("a")),
            entry(4, "summarise", None),
            entry(5, "_custom", Some("B")),
        ];
        assert!(last_undoable(&h).is_none()); // nothing was pasted
        h[2].replaced = true;
        assert_eq!(last_undoable(&h).unwrap().ts, 2);
        assert_eq!(recent_prompts(&h, 20), ["B", "a"]);
        assert_eq!(recent_prompts(&h, 1), ["B"]);
    }
}
