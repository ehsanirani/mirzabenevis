//! `mirzabenevis-cli doctor`: what works on this desktop, what is missing and
//! how to bind the key. Read-only: it never installs or writes anything.

use mirzabenevis_core::text::{self, PasteResult};
use mirzabenevis_core::{backends, Config};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Distro {
    Nixos,
    Debian,
    Fedora,
    Arch,
    Other,
}

/// From the contents of `/etc/os-release` (`ID` and `ID_LIKE`).
pub fn distro(os_release: &str) -> Distro {
    let ids: Vec<&str> = os_release
        .lines()
        .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
        .flat_map(|v| v.trim_matches('"').split_whitespace())
        .collect();
    let has = |names: &[&str]| ids.iter().any(|id| names.contains(id));
    if has(&["nixos"]) {
        Distro::Nixos
    } else if has(&["debian", "ubuntu"]) {
        Distro::Debian
    } else if has(&["fedora", "rhel"]) {
        Distro::Fedora
    } else if has(&["arch"]) {
        Distro::Arch
    } else {
        Distro::Other
    }
}

/// How to install the package that provides `tool`.
pub fn install_hint(distro: Distro, tool: &str) -> String {
    let pkg = match tool {
        "wl-paste" | "wl-copy" => "wl-clipboard",
        "notify-send" if distro == Distro::Debian => "libnotify-bin",
        "notify-send" => "libnotify",
        t => t,
    };
    match (distro, pkg) {
        (Distro::Nixos, "ydotool") => {
            "programs.ydotool.enable = true; and add yourself to the ydotool group".into()
        }
        (Distro::Nixos, _) => format!("add pkgs.{pkg} to environment.systemPackages"),
        (Distro::Debian, _) => format!("sudo apt install {pkg}"),
        (Distro::Fedora, _) => format!("sudo dnf install {pkg}"),
        (Distro::Arch, _) => format!("sudo pacman -S {pkg}"),
        (Distro::Other, _) => format!("install {pkg}"),
    }
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

pub const AUTOSTART: &str = "\
[Desktop Entry]
Type=Application
Name=mirzabenevis
Exec=mirzabenevis
Icon=mirzabenevis
X-GNOME-Autostart-enabled=true
";

/// Prints the report; false when grabbing or pasting cannot work.
pub async fn run(cfg: &Config) -> bool {
    let env = |k| std::env::var(k).unwrap_or_default();
    let desk = desktop(&env("XDG_CURRENT_DESKTOP"));
    let distro = distro(&std::fs::read_to_string("/etc/os-release").unwrap_or_default());
    let wayland = text::wayland();
    println!(
        "Desktop: {}, {}\n",
        or_unknown(&env("XDG_CURRENT_DESKTOP")),
        if wayland { "Wayland" } else { "X11" }
    );

    let mut ok = true;
    let row = |mark: &str, what: &str, detail: &str| println!("  {mark} {what:<15} {detail}");
    println!("Tools");

    match text::first_available(&cfg.linux.grab_cmds) {
        Some(cmd) => row("✓", "grab selection", &cmd.join(" ")),
        None => {
            ok = false;
            let tool = if wayland { "wl-paste" } else { "xclip" };
            row("✗", "grab selection", &install_hint(distro, tool));
        }
    }

    if wayland {
        match text::on_path("wl-copy") {
            true => row("✓", "clipboard", "wl-copy"),
            false => row(
                "!",
                "clipboard",
                &format!(
                    "no wl-copy: copied text is lost when the app exits; {}",
                    install_hint(distro, "wl-copy")
                ),
            ),
        }
    }

    let method = text::paste_method(cfg);
    let tool = match method {
        PasteResult::Pasted => text::first_available(&cfg.linux.paste_key_cmds),
        PasteResult::Typed => text::first_available(&cfg.linux.paste_cmds),
        PasteResult::Clipboard => None,
    };
    let tool_name = tool.map_or("", |c| c[0].rsplit('/').next().unwrap_or(""));
    // wtype needs a protocol GNOME does not implement.
    let gnome_wtype = desk == Desktop::Gnome && wayland && tool_name == "wtype";
    if tool.is_none() || (gnome_wtype && !text::on_path("ydotool")) {
        let want = match (wayland, desk) {
            (true, Desktop::Gnome | Desktop::Kde) => "ydotool",
            (true, _) => "wtype",
            (false, _) => "xdotool",
        };
        let detail = format!("results can only be copied; {}", install_hint(distro, want));
        if cfg.general.auto_paste {
            ok = false;
            row("✗", "paste", &detail);
        } else {
            row("!", "paste", &format!("{detail} (auto_paste is off)"));
        }
    } else if let Some(cmd) = tool {
        let how = match method {
            PasteResult::Typed => "types the text",
            _ => "clipboard + Ctrl+V",
        };
        row("✓", "paste", &format!("{} ({how})", cmd[0]));
    }

    if text::on_path("ydotool") {
        match ydotool_socket() {
            Some(s) => row("✓", "ydotoold", &s),
            None => row(
                "!",
                "ydotoold",
                match distro {
                    Distro::Nixos => {
                        "not running or no access: programs.ydotool.enable = true; and the ydotool group"
                    }
                    _ => "not running or no access: start ydotoold (systemd unit) and check the socket permissions",
                },
            ),
        }
    }

    match text::on_path("notify-send") {
        true => row("✓", "notifications", "notify-send"),
        false => row(
            "!",
            "notifications",
            &format!(
                "errors from a keybind go unseen; {}",
                install_hint(distro, "notify-send")
            ),
        ),
    }

    println!("\nAI backends");
    let det = backends::detect_current(cfg, true).await;
    super::print_providers(cfg, &det);

    println!("\nKey binding\n{}", keybinding_help(desk));
    println!("Start the tray on login: save this as ~/.config/autostart/mirzabenevis.desktop\n{AUTOSTART}");
    if !ok {
        println!("Something needed is missing (✗ above).");
    }
    ok
}

fn or_unknown(s: &str) -> &str {
    if s.is_empty() {
        "unknown"
    } else {
        s
    }
}

/// The ydotoold socket we can write to, if any (same lookup as the ydotool client).
fn ydotool_socket() -> Option<String> {
    let candidates = match std::env::var("YDOTOOL_SOCKET") {
        Ok(s) => vec![s],
        Err(_) => {
            let run = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
            vec![
                format!("{run}/.ydotool_socket"),
                "/tmp/.ydotool_socket".into(),
            ]
        }
    };
    candidates.into_iter().find(|p| {
        let Ok(c) = std::ffi::CString::new(p.as_str()) else {
            return false;
        };
        // SAFETY: c is a valid NUL-terminated path.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_from_env() {
        assert_eq!(desktop("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(desktop("GNOME"), Desktop::Gnome);
        assert_eq!(desktop("KDE"), Desktop::Kde);
        assert_eq!(desktop("Hyprland"), Desktop::Hyprland);
        assert_eq!(desktop("sway"), Desktop::Sway);
        assert_eq!(desktop(""), Desktop::Other);
        assert_eq!(desktop("niri"), Desktop::Other);
    }

    #[test]
    fn distro_and_hints() {
        let ubuntu = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n";
        assert_eq!(distro(ubuntu), Distro::Debian);
        assert_eq!(distro("ID=nixos\n"), Distro::Nixos);
        assert_eq!(
            distro("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"),
            Distro::Fedora
        );
        assert_eq!(distro("ID=endeavouros\nID_LIKE=arch\n"), Distro::Arch);
        assert_eq!(distro(""), Distro::Other);
        assert_eq!(
            install_hint(Distro::Debian, "notify-send"),
            "sudo apt install libnotify-bin"
        );
        assert_eq!(
            install_hint(Distro::Arch, "wl-paste"),
            "sudo pacman -S wl-clipboard"
        );
        assert!(install_hint(Distro::Nixos, "ydotool").contains("programs.ydotool.enable"));
    }

    #[test]
    fn snippets_open_the_popup() {
        for d in [
            Desktop::Gnome,
            Desktop::Kde,
            Desktop::Hyprland,
            Desktop::Sway,
        ] {
            assert!(keybinding_help(d).contains("mirzabenevis --popup"), "{d:?}");
        }
        assert!(keybinding_help(Desktop::Other).contains("bindsym $mod+g"));
        assert!(keybinding_help(Desktop::Gnome).contains("gsettings set $K binding '<Super>g'"));
        assert!(AUTOSTART.contains("\nExec=mirzabenevis\n"));
    }
}
