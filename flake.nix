{
  description = "mirzabenevis — select text anywhere, press a key, get it rewritten";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.crane.url = "github:ipetkov/crane";

  outputs = { self, nixpkgs, crane, ... }:
    let
      forAllSystems = f: nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" ]
        (system: f nixpkgs.legacyPackages.${system});
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
      src = nixpkgs.lib.fileset.toSource {
        root = ./.;
        fileset = nixpkgs.lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./config.example.toml
          ./crates
          ./src-tauri
          ./tests-e2e # a core test uses fake_agent.sh
          ./ui
        ];
      };
      meta = {
        description = "Select text anywhere, press a key, get it fixed, rewritten or translated";
        homepage = "https://github.com/ehsanirani/mirzabenevis";
        license = nixpkgs.lib.licenses.mit;
        platforms = nixpkgs.lib.platforms.linux;
      };
    in
    {
      packages = forAllSystems (pkgs:
        let
          craneLib = crane.mkLib pkgs;
          # Shared by the deps-only build and the final builds, so the
          # cached dependency artifacts match and are reused.
          commonArgs = {
            inherit src version;
            pname = "mirzabenevis";
            strictDeps = true;
            cargoExtraArgs = "-p mirzabenevis -p mirzabenevis-cli --features mirzabenevis/custom-protocol";
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = with pkgs; [ webkitgtk_4_1 dbus libayatana-appindicator ];
          };
          # All third-party crates, built once. Only changes when Cargo.lock does.
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          # The tray app + popup + CLI.
          default = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            doCheck = false; # core + cli tests run in the cli package
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.wrapGAppsHook3 ];
            # The tray library is dlopen()ed by name.
            preFixup = ''
              gappsWrapperArgs+=(--prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ]})
            '';
            postInstall = ''
              for s in 32x32 128x128; do
                install -Dm644 src-tauri/icons/$s.png $out/share/icons/hicolor/$s/apps/mirzabenevis.png
              done
              install -Dm644 src-tauri/icons/128x128@2x.png $out/share/icons/hicolor/256x256/apps/mirzabenevis.png
              install -Dm644 src-tauri/icons/icon.png $out/share/icons/hicolor/512x512/apps/mirzabenevis.png
              install -Dm644 /dev/stdin $out/share/applications/mirzabenevis.desktop <<EOF
              [Desktop Entry]
              Categories=Utility;
              Comment=Select text anywhere, press a key, get it rewritten
              Exec=mirzabenevis
              StartupWMClass=mirzabenevis
              Icon=mirzabenevis
              Name=mirzabenevis
              Terminal=false
              Type=Application
              EOF
            '';
            meta = meta // { mainProgram = "mirzabenevis"; };
          });

          # Just the headless CLI, no GTK.
          cli = craneLib.buildPackage {
            inherit src version;
            pname = "mirzabenevis-cli";
            strictDeps = true;
            cargoExtraArgs = "-p mirzabenevis-cli";
            cargoTestExtraArgs = "-p mirzabenevis-core";
            # Agent backends run in a work dir under the cache dir; one test uses ps.
            preCheck = "export HOME=$TMPDIR";
            nativeCheckInputs = [ pkgs.procps ];
            meta = meta // { mainProgram = "mirzabenevis-cli"; };
          };
        });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/mirzabenevis";
          meta.description = meta.description;
        };
      });

      checks = forAllSystems (pkgs: {
        default = pkgs.runCommand "mirzabenevis-package-check" { } ''
          p=${self.packages.${pkgs.stdenv.hostPlatform.system}.default}
          $p/bin/mirzabenevis-cli --version
          grep -qx 'Exec=mirzabenevis' $p/share/applications/mirzabenevis.desktop
          touch $out
        '';
      });

      # programs.mirzabenevis for Home Manager. See docs/install.md.
      homeManagerModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.mirzabenevis;
          kb = "org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/mirzabenevis";
        in
        {
          options.programs.mirzabenevis = {
            enable = lib.mkEnableOption "mirzabenevis, the select-and-rewrite popup";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              description = "The mirzabenevis package (tray app + CLI).";
            };
            keybinding = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = "<Super>g";
              description = ''
                GNOME shortcut that opens the popup, null for none. Only the
                shortcut itself is written: add its path to your
                custom-keybindings list (see docs/install.md).
              '';
            };
            autostart = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Start the tray app on login (XDG autostart).";
            };
          };

          config = lib.mkIf cfg.enable (lib.mkMerge [
            { home.packages = [ cfg.package ]; }
            (lib.mkIf (cfg.keybinding != null) {
              dconf.settings.${kb} = {
                name = "mirzabenevis";
                command = "${lib.getExe cfg.package} --popup";
                binding = cfg.keybinding;
              };
            })
            (lib.mkIf cfg.autostart {
              xdg.autostart.enable = true;
              xdg.autostart.entries = [ "${cfg.package}/share/applications/mirzabenevis.desktop" ];
            })
          ]);
        };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ cargo rustc clippy rustfmt cargo-tauri pkg-config ];
          # What the Tauri app links against (found via pkg-config).
          # webkitgtk_4_1 brings gtk3, libsoup_3, glib, cairo, pango with it.
          # libayatana-appindicator: `cargo tauri build` looks it up for the deb.
          buildInputs = with pkgs; [ webkitgtk_4_1 dbus libayatana-appindicator ];
          # The tray icon library is dlopen()ed at runtime, not linked.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ];
        };
      });
    };
}
