{
  description = "mirzabenevis — select text anywhere, press a key, get it rewritten";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs, ... }:
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
      packages = forAllSystems (pkgs: {
        # The tray app + popup. The deb bundle also carries mirzabenevis-cli
        # (tauri.conf.json), so this installs both binaries.
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "mirzabenevis";
          inherit version src;
          cargoLock.lockFile = ./Cargo.lock;
          cargoRoot = ".";
          buildAndTestSubdir = "src-tauri";
          # The tray library is dlopen()ed by name: point it at the store path.
          postPatch = ''
            substituteInPlace $cargoDepsCopy/libappindicator-sys-*/src/lib.rs \
              --replace-fail "libayatana-appindicator3.so.1" "${pkgs.libayatana-appindicator}/lib/libayatana-appindicator3.so.1"
          '';
          # The hook sets CARGO_BUILD_TARGET, so beforeBuildCommand's CLI lands in
          # target/<triple>/release; point the deb's file mapping there.
          preBuild = ''
            mkdir -p target/release
            ln -s ../${pkgs.stdenv.hostPlatform.rust.cargoShortTarget}/release/mirzabenevis-cli target/release/
          '';
          nativeBuildInputs = with pkgs; [ cargo-tauri.hook pkg-config wrapGAppsHook3 ];
          buildInputs = with pkgs; [ webkitgtk_4_1 dbus libayatana-appindicator ];
          meta = meta // { mainProgram = "mirzabenevis"; };
        };

        # Just the headless CLI, no GTK.
        cli = pkgs.rustPlatform.buildRustPackage {
          pname = "mirzabenevis-cli";
          inherit version src;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "mirzabenevis-cli" ];
          cargoTestFlags = [ "-p" "mirzabenevis-core" "-p" "mirzabenevis-cli" ];
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
