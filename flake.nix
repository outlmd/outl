# Nix flake for outl. Full guide: docs/nix.md.
#
#   nix run github:outlmd/outl        # CLI/TUI/MCP
#   nix run github:outlmd/outl#outl-desktop
#
# Linux packages: `outl` and `outl-desktop`.
# Home Manager module: `homeManagerModules.default`.
{
  description = "outl - local-first outliner with CRDT sync (Nix packages and Home Manager module)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      home-manager,
    }:
    # Packages are built for Linux. Home Manager also manages the
    # config file on macOS, where stable Tauri SDKs are unavailable.

    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
      ]
      (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };

          # Build the source tree represented by this flake input.
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          version =
            (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version or "0.12.0";
          projectSrc = pkgs.lib.cleanSourceWith {
            src = self;
            filter =
              path: type:
              let
                name = pkgs.lib.baseNameOf path;
              in
              !builtins.elem name [
                ".git"
                ".github"
                "devenv.lock"
                "devenv.nix"
                "devenv.yaml"
                "flake.lock"
                "flake.nix"
                "hm-module.nix"
                "target"
              ];
          };
          linuxDeps = with pkgs; [
            webkitgtk_4_1
            gtk3
            cairo
            gdk-pixbuf
            glib
            dbus
            openssl_3
            libsoup_3
            librsvg
            libappindicator-gtk3
          ];

          mkOutl =
            {
              pname,
              version,
              src,
              cargoLockFile,
              rust,
            }:
            pkgs.rustPlatform.buildRustPackage {
              inherit pname version src;

              cargoLock.lockFile = cargoLockFile;

              nativeBuildInputs = with pkgs; [
                rust
                pkg-config
              ];

              # The CLI and TUI are terminal apps with no system-library
              # dependency: sync uses rustls (never openssl/native-tls) and the
              # TUI is not Tauri, so the GTK/WebKit closure the desktop needs is
              # irrelevant here.
              buildInputs = [ ];

              cargoBuildFlags = [
                "-p"
                "outl-cli"
                "-p"
                "outl-tui"
              ];

              doCheck = false;

              meta = with pkgs.lib; {
                description = "Local-first outliner with CRDT sync";
                homepage = "https://outl.app";
                license = licenses.mit;
                mainProgram = "outl";
                platforms = platforms.linux;
              };
            };

          mkDesktopFrontend =
            {
              pname,
              version,
              src,
              outputHash,
            }:
            pkgs.stdenv.mkDerivation {
              inherit pname version src;

              nativeBuildInputs = with pkgs; [
                bun
                nodejs
              ];

              # Fixed-output derivation: update hash when the frontend source
              # changes. Run the build and copy the `got: sha256-...` value
              # from the mismatch error into `outputHash`.
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
              inherit outputHash;

              buildPhase = ''
                export HOME=$TMPDIR
                cd crates/outl-desktop
                bun install --frozen-lockfile
                # `bun run build` execs node_modules/.bin/vite, whose shebang is
                # `#!/usr/bin/env node` and the Nix sandbox has no /usr/bin/env,
                # so it dies with "bad interpreter". bun materialises .bin entries
                # as symlinks whose targets may live in its global cache OUTSIDE
                # node_modules (what the GitHub runners hit; a warm local cache
                # links inside the tree, which is why the same derivation builds
                # locally yet fails in CI). patchShebangs skips symlinks and, when
                # the target is out of tree, never reaches it. Resolve each .bin
                # symlink to its real file and patch THAT in place -- its path is
                # preserved, so the script's own relative imports (vite resolves
                # ../dist via the symlink's realpath) still work. installPhase only
                # copies dist/, so the output hash is unaffected.
                for f in node_modules/.bin/*; do
                  if [ -L "$f" ]; then
                    t="$(readlink -f "$f")"
                    [ -w "$t" ] || chmod u+w "$t"
                    patchShebangs "$t"
                  fi
                done
                patchShebangs --build node_modules
                bun run build
              '';

              installPhase = ''
                mkdir -p $out
                cp -r dist/* $out/
              '';
            };

          mkOutlDesktop =
            {
              pname,
              version,
              src,
              cargoLockFile,
              rust,
              frontend,
              features,
            }:
            pkgs.rustPlatform.buildRustPackage {
              inherit pname version src;

              cargoLock.lockFile = cargoLockFile;

              buildAndTestSubdir = "crates/outl-desktop/src-tauri";

              # Plain cargo build skips Tauri's bundler, so enable the custom
              # protocol feature to embed frontendDist instead of loading Vite.
              cargoBuildFlags = [
                "--features"
                (builtins.concatStringsSep "," features)
              ];

              nativeBuildInputs = with pkgs; [
                rust
                pkg-config
                makeWrapper
                wrapGAppsHook3
                gobject-introspection
                desktop-file-utils
                xdg-utils
                jq
              ];

              buildInputs = linuxDeps;

              preBuild = ''
                mkdir -p crates/outl-desktop/dist
                cp -r ${frontend}/* crates/outl-desktop/dist/
              '';

              # Plain cargo build skips Tauri's bundler, which normally emits
              # the .desktop entry + hicolor icons. Emit them into standard XDG
              # paths for NixOS / Home Manager. Read metadata at build time:
              # interpolating `${src}` is pure-eval safe, while builtins.readFile
              # would try to realize the source during flake evaluation.
              postInstall = ''
                mkdir -p $out/share/applications

                SRC=${src}/crates/outl-desktop/src-tauri
                ID=$(jq -r .identifier "$SRC/tauri.conf.json")
                NAME=$(jq -r .productName "$SRC/tauri.conf.json")
                MIME=$(jq -r '[(.bundle.fileAssociations // [])[].mimeType] | join(";") + ";"' "$SRC/tauri.conf.json")

                cat > "$out/share/applications/$ID.desktop" <<DESKTOP
                [Desktop Entry]
                Type=Application
                Version=1.0
                Name=$NAME
                GenericName=Outliner
                Comment=Local-first outliner with CRDT sync
                Exec=$out/bin/outl-desktop %u
                Icon=$ID
                Terminal=false
                StartupNotify=true
                Categories=Utility;TextEditor;
                Keywords=outline;notes;journal;markdown;outliner;
                StartupWMClass=outl-desktop
                MimeType=$MIME
                DESKTOP

                desktop-file-validate "$out/share/applications/$ID.desktop"

                install -Dm644 "$SRC/icons/32x32.png"     "$out/share/icons/hicolor/32x32/apps/$ID.png"
                install -Dm644 "$SRC/icons/128x128.png"    "$out/share/icons/hicolor/128x128/apps/$ID.png"
                install -Dm644 "$SRC/icons/128x128@2x.png" "$out/share/icons/hicolor/256x256/apps/$ID.png"
                # Deliberately no `hicolor/scalable/apps/$ID.svg`. Tauri ships a
                # placeholder `icon.svg` (a `>_` terminal glyph), and scalable-
                # first icon lookups (rofi drun, GTK) load it ahead of the brand
                # PNGs — so shipping it made the launcher show a generic
                # terminal. Omit it and the lookup falls through to the brand
                # PNG sizes installed above.

                wrapArgs=( --prefix PATH : "${pkgs.desktop-file-utils}/bin:${pkgs.xdg-utils}/bin" )
                [ -n "$GI_TYPELIB_PATH" ] && wrapArgs+=( --prefix GI_TYPELIB_PATH : "$GI_TYPELIB_PATH" )
                [ -n "$GST_PLUGIN_PATH" ] && wrapArgs+=( --prefix GST_PLUGIN_PATH : "$GST_PLUGIN_PATH" )
                wrapProgram $out/bin/outl-desktop "''${wrapArgs[@]}"
              '';

              doCheck = false;

              meta = with pkgs.lib; {
                description = "Desktop client for outl (Tauri 2)";
                homepage = "https://outl.app";
                license = licenses.mit;
                mainProgram = "outl-desktop";
                platforms = platforms.linux;
              };
            };

          outl = mkOutl {
            pname = "outl";
            inherit version;
            src = projectSrc;
            cargoLockFile = ./Cargo.lock;
            rust = rustToolchain;
          };

          desktopFrontend = mkDesktopFrontend {
            pname = "outl-desktop-frontend";
            inherit version;
            src = projectSrc;
            outputHash = "sha256-oi6tiLpbxGeyjAQ4D5vBkLrzx3cDRisYMbRRYey9RgU=";
          };

          outlDesktop = mkOutlDesktop {
            pname = "outl-desktop";
            inherit version;
            src = projectSrc;
            cargoLockFile = ./Cargo.lock;
            rust = rustToolchain;
            frontend = desktopFrontend;
            features = [ "tauri/custom-protocol" ];
          };

          homeManagerConfig = home-manager.lib.homeManagerConfiguration {
            inherit pkgs;
            modules = [
              self.homeManagerModules.default
              {
                home.username = "outl-nix-check";
                home.homeDirectory = "/home/outl-nix-check";
                home.stateVersion = "26.05";
                programs.outl = {
                  enable = true;
                  settings = {
                    managed = true;
                    workspace.last = "/home/outl-nix-check/notes";
                  };
                  services.sync = {
                    enable = true;
                    workspace = "/home/outl-nix-check/notes";
                  };
                };
              }
            ];
          };

          homeManagerConfigFile = homeManagerConfig.config.xdg.configFile."outl/config.toml".source;
          homeManagerModuleCheck =
            assert homeManagerConfig.config.programs.outl.settings.managed;
            assert builtins.hasAttr "outl-sync" homeManagerConfig.config.systemd.user.services;
            pkgs.runCommand "outl-home-manager-module" { } ''
              ${pkgs.gnugrep}/bin/grep -q '^managed = true$' ${homeManagerConfigFile}
              touch "$out"
            '';
        in
        {
          packages = {
            inherit outl;
            outl-desktop = outlDesktop;
            outl-desktop-frontend = desktopFrontend;
            default = outl;
          };

          checks.home-manager-module = homeManagerModuleCheck;

          devShells.default = pkgs.mkShell {
            inputsFrom = [
              outl
              outlDesktop
            ];
            packages = with pkgs; [
              rustToolchain
              cargo-tauri
              bun
              nodejs
              just
            ];
          };

          formatter = pkgs.nixfmt;
        }
      )
    // {
      homeManagerModules.default = import ./hm-module.nix self;
    };
}
