{
  description = "mirzabenevis — select text anywhere, press a key, get it rewritten";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, ... }:
    let
      forAllSystems = f: nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" ]
        (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ cargo rustc clippy rustfmt cargo-tauri pkg-config ];
          # What the Tauri app links against (found via pkg-config).
          # webkitgtk_4_1 brings gtk3, libsoup_3, glib, cairo, pango with it.
          buildInputs = with pkgs; [ webkitgtk_4_1 dbus ];
          # The tray icon library is dlopen()ed at runtime, not linked.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ];
        };
      });
    };
}
