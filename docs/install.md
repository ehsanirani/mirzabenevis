# Install

Back to the [README](../README.md).

You get two programs: `mirzabenevis` (tray app + popup) and `mirzabenevis-cli`
(headless; bind it to a key or pipe text through it). After installing, run
`mirzabenevis-cli doctor`: it checks the grab/paste tools and AI backends and prints
the exact key-binding and autostart lines for your desktop.

## NixOS (flake)

Add the flake as an input and the package to your system:

```nix
# flake.nix
inputs.mirzabenevis.url = "github:ehsanirani/mirzabenevis";

# configuration.nix (with `inputs` passed in via specialArgs)
environment.systemPackages = [ inputs.mirzabenevis.packages.${pkgs.stdenv.hostPlatform.system}.default pkgs.wl-clipboard ];
programs.ydotool.enable = true;                  # pastes back on GNOME/KDE Wayland
users.users.<you>.extraGroups = [ "ydotool" ];   # log out and in after this
```

`packages.default` has both binaries; `packages.cli` is the CLI alone (no GTK).
Try it without installing: `nix run github:ehsanirani/mirzabenevis -- --popup`.

## Home Manager

```nix
imports = [ inputs.mirzabenevis.homeManagerModules.default ];
programs.mirzabenevis = {
  enable = true;
  keybinding = "<Super>g";   # GNOME shortcut, null for none
  autostart = true;          # tray app on login
};
```

The ydotool lines above still go in the NixOS configuration: Home Manager cannot
start system daemons or change groups.

**GNOME shortcut caveat.** The module writes the shortcut itself
(`/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/mirzabenevis/`) but not
the list of active custom shortcuts, because setting that list would replace yours. Add the
path to it once, in the same Home Manager config:

```nix
dconf.settings."org/gnome/settings-daemon/plugins/media-keys".custom-keybindings = [
  "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/mirzabenevis/"
  # ...your other custom shortcuts, if any
];
```

On Hyprland or Sway, bind `mirzabenevis --popup` yourself (`doctor` prints the line).

## Debian, Ubuntu (.deb) and other distros (AppImage)

Download the `.deb` or `.AppImage` from the
[Releases](https://github.com/ehsanirani/mirzabenevis/releases) page. They are built on
Ubuntu 24.04 and need glibc 2.39 or newer (Ubuntu 24.04+, Debian 13+, Fedora 40+).

```sh
sudo apt install ./mirzabenevis_*_amd64.deb   # pulls in webkit, the tray library, wl-clipboard, ydotool
```

The `.deb` contains `mirzabenevis-cli` too; the AppImage has only the tray app.
On GNOME/KDE Wayland pasting needs the `ydotoold` daemon running and access to its
socket (usually: enable the `ydotool` systemd service and add yourself to the group that
owns the socket). Then bind `mirzabenevis --popup` to a key: `mirzabenevis-cli doctor`
prints how.

## From source

```sh
git clone https://github.com/ehsanirani/mirzabenevis && cd mirzabenevis
nix develop                          # or install Rust, the Tauri prerequisites and cargo-tauri
cargo tauri build --bundles deb      # target/release/bundle/deb/*.deb
cargo install --path crates/cli      # just the CLI
```

## GNOME tray icon

GNOME hides tray icons unless the
[AppIndicator](https://extensions.gnome.org/extension/615/appindicator-support/) extension
is enabled (Ubuntu ships it). Without it the app still works: the popup opens on your
shortcut, there is just no tray menu.
