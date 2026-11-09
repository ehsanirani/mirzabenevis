//! The key that opens the popup: what to bind on each desktop, and on GNOME
//! setting it directly (a custom shortcut in gsettings, next to the user's own).

use anyhow::{bail, Context, Result};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Desktop {
    Gnome,
    Kde,
    Hyprland,
    Sway,
    Other,
}

/// From `XDG_CURRENT_DESKTOP`, e.g. `ubuntu:GNOME`.
pub fn desktop(xdg_current_desktop: &str) -> Desktop {
    for part in xdg_current_desktop.split(':') {
        match part.to_ascii_lowercase().as_str() {
            "gnome" => return Desktop::Gnome,
            "kde" => return Desktop::Kde,
            "hyprland" => return Desktop::Hyprland,
            "sway" => return Desktop::Sway,
            _ => {}
        }
    }
    Desktop::Other
}

/// Where to bind `mirzabenevis --popup` on this desktop (all of them when unknown).
pub fn keybinding_help(d: Desktop) -> String {
    const GNOME: &str = "\
GNOME: Settings → Keyboard → View and Customise Shortcuts → Custom Shortcuts → +
  Name: mirzabenevis   Command: mirzabenevis --popup   Shortcut: Super+G
Or from a terminal. The first line replaces your list of custom shortcuts: if
`gsettings get org.gnome.settings-daemon.plugins.media-keys custom-keybindings`
is not empty, add the path to that list instead.
  gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings \"['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/mirzabenevis/']\"
  K=org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/mirzabenevis/
  gsettings set $K name mirzabenevis
  gsettings set $K command 'mirzabenevis --popup'
  gsettings set $K binding '<Super>g'
";
    const KDE: &str = "\
KDE: System Settings → Keyboard → Shortcuts → Add New → Command or Script…
  Command: mirzabenevis --popup
";
    const HYPRLAND: &str = "\
Hyprland (~/.config/hypr/hyprland.conf):
  bind = SUPER, G, exec, mirzabenevis --popup
";
    const SWAY: &str = "\
Sway (~/.config/sway/config):
  bindsym $mod+g exec mirzabenevis --popup
";
    match d {
        Desktop::Gnome => GNOME.into(),
        Desktop::Kde => KDE.into(),
        Desktop::Hyprland => HYPRLAND.into(),
        Desktop::Sway => SWAY.into(),
        Desktop::Other => [GNOME, KDE, HYPRLAND, SWAY].join("\n"),
    }
}

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
/// Same path as the Home Manager module, so both edit one shortcut.
const GNOME_PATH: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/mirzabenevis/";

fn gsettings(args: &[&str]) -> Result<String> {
    let out = Command::new("gsettings")
        .args(args)
        .output()
        .context("gsettings is not available")?;
    if !out.status.success() {
        bail!(
            "gsettings {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn our_key() -> String {
    format!("{MEDIA_KEYS}.custom-keybinding:{GNOME_PATH}")
}

/// The paths in a printed `as` list: `@as []` or `['/a/', '/b/']`.
fn parse_list(printed: &str) -> Vec<String> {
    printed
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn format_list(paths: &[String]) -> String {
    let quoted: Vec<String> = paths.iter().map(|p| gvariant_str(p)).collect();
    format!("[{}]", quoted.join(", "))
}

fn gvariant_str(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn gnome_list() -> Result<Vec<String>> {
    Ok(parse_list(&gsettings(&[
        "get",
        MEDIA_KEYS,
        "custom-keybindings",
    ])?))
}

/// The accelerator (e.g. `<Super>g`) of our GNOME shortcut, if it is active.
pub fn gnome_get() -> Result<Option<String>> {
    if !gnome_list()?.iter().any(|p| p == GNOME_PATH) {
        return Ok(None);
    }
    let b = gsettings(&["get", &our_key(), "binding"])?;
    let b = parse_list(&b).pop().unwrap_or_default();
    Ok((!b.is_empty()).then_some(b))
}

/// Bind `accel` to `command`, adding our shortcut to the user's list if needed.
pub fn gnome_set(accel: &str, command: &str) -> Result<()> {
    if accel.trim().is_empty() {
        bail!("no key given");
    }
    let k = our_key();
    gsettings(&["set", &k, "name", "'mirzabenevis'"])?;
    gsettings(&["set", &k, "command", &gvariant_str(command)])?;
    gsettings(&["set", &k, "binding", &gvariant_str(accel)])?;
    let mut list = gnome_list()?;
    if !list.iter().any(|p| p == GNOME_PATH) {
        list.push(GNOME_PATH.into());
        gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &format_list(&list)])?;
    }
    Ok(())
}

/// Take our shortcut out of the user's list; their other shortcuts stay.
pub fn gnome_remove() -> Result<()> {
    let list: Vec<String> = gnome_list()?
        .into_iter()
        .filter(|p| p != GNOME_PATH)
        .collect();
    gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &format_list(&list)])?;
    gsettings(&["reset", &our_key(), "binding"])?;
    Ok(())
}

/// Keys GNOME itself binds; a schema missing on this GNOME version is skipped.
const GNOME_KEY_SCHEMAS: [&str; 5] = [
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    MEDIA_KEYS,
];

/// Whether GNOME or another custom shortcut already uses `accel`.
// ponytail: exact text match, so `<Alt><Control>g` is not seen as `<Control><Alt>g`.
fn gnome_taken(accel: &str) -> Result<bool> {
    let mut used = String::new();
    for schema in GNOME_KEY_SCHEMAS {
        used += &gsettings(&["list-recursively", schema]).unwrap_or_default();
    }
    for p in gnome_list()?.iter().filter(|p| *p != GNOME_PATH) {
        let k = format!("{MEDIA_KEYS}.custom-keybinding:{p}");
        used += &gsettings(&["get", &k, "binding"]).unwrap_or_default();
    }
    Ok(used
        .to_lowercase()
        .contains(&gvariant_str(&accel.to_lowercase())))
}

/// Our shortcut, set on first run to the first default key nothing else uses.
/// `None` when every default is taken.
pub fn gnome_default(command: &str) -> Result<Option<String>> {
    if let Some(b) = gnome_get()? {
        return Ok(Some(b));
    }
    for accel in ["<Super>g", "<Control><Alt>g"] {
        if !gnome_taken(accel)? {
            gnome_set(accel, command)?;
            return Ok(Some(accel.into()));
        }
    }
    Ok(None)
}

/// The command a shortcut runs: this program by absolute path (the AppImage
/// itself when run from one), so it works whatever GNOME's PATH is.
pub fn popup_command() -> String {
    let exe = std::env::var("APPIMAGE")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| Some(std::env::current_exe().ok()?.to_str()?.to_string()));
    match exe {
        Some(p) => format!("\"{p}\" --popup"),
        None => "mirzabenevis --popup".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_list_round_trip() {
        assert!(parse_list("@as []").is_empty());
        let l = parse_list("['/a/custom0/', '/b/']");
        assert_eq!(l, ["/a/custom0/", "/b/"]);
        assert_eq!(format_list(&l), "['/a/custom0/', '/b/']");
        assert_eq!(format_list(&[]), "[]");
        assert_eq!(gvariant_str(r"it's \x"), r"'it\'s \\x'");
    }
}
