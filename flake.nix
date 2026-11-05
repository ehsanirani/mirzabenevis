{
  description = "mirzabenevis — select text anywhere, press a key, get it rewritten";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # Native libraries Tauri 2 links against on Linux.
        tauriLibs = with pkgs; [
          webkitgtk_4_1
          gtk3
          libsoup_3
          glib
          glib-networking   # TLS for webkit; needs GIO_EXTRA_MODULES below
          cairo
          pango
          gdk-pixbuf
          librsvg
          openssl
          libayatana-appindicator  # tray icon
          xdotool                  # global-hotkey crate on X11
        ];

        # Helpers the CLI / core shell out to for grabbing and pasting text.
        # All optional and configurable in config.toml.
        textTools = with pkgs; [
          wl-clipboard  # wl-paste / wl-copy   (Wayland)
          wtype         # type text            (wlroots compositors)
          ydotool       # type text            (any Wayland, needs daemon)
          xclip         # clipboard            (X11)
          xdotool       # type text / keys     (X11)
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            cargo-tauri
            pkg-config
            nodejs_22          # only needed if you later add a JS bundler
          ] ++ tauriLibs ++ textTools;

          # Let webkit find TLS backends (fixes "TLS/SSL support not available").
          GIO_EXTRA_MODULES = "${pkgs.glib-networking}/lib/gio/modules";
          # Helps on NVIDIA + some Wayland setups where the webview renders blank.
          WEBKIT_DISABLE_COMPOSITING_MODE = "1";
          # Tauri looks for the appindicator lib at runtime.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ];

          shellHook = ''
            echo "mirzabenevis dev shell"
            echo "  cargo run -p mirzabenevis-cli -- --help   # headless CLI"
            echo "  cargo tauri dev                          # tray app + popup"
          '';
        };
      });
}
