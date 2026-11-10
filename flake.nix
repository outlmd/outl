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
            openssl
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

          # The frontend build is split so CI can keep the strict Nix
          # sandbox (nix-build.yml):
          #
          #   1. `mkDesktopNodeModules` is a fixed-output derivation running
          #      `bun install` at the workspace root. FODs are the one
          #      sandbox exception: they get network through it, so the npm
          #      tarball fetches work with `sandbox = true`. The output is a
          #      self-contained node_modules tree addressed by its declared
          #      hash. `--linker=copyfile` writes real files instead of
          #      hardlinks into bun's global content store, so the tree is
          #      closed over — nothing downstream dereferences outside $out.
          #   2. `mkDesktopFrontend` is a normal derivation running `vite
          #      build` offline against that tree. A normal derivation gets
          #      no network even with `allowNetworking = true`, which is
          #      fine: the bundle step only reads vendored files. Being
          #      non-FOD, its output path depends on src, so a frontend edit
          #      rebuilds the bundle (a FOD would address the stale bundle
          #      by its declared hash and skip Vite).
          #
          # Changing bun.lock invalidates the deps hash: `nix build
          # .#outl-desktop-node-modules` then fails and prints the new value
          # as `got: sha256-...`; put it in frontendNodeModulesHash below.
          # The hash is per-system because node_modules carries native
          # binaries (esbuild, tailwind-oxide) built for the build machine.
          frontendNodeModulesHash = {
            x86_64-linux = "sha256-hXPno3IaCc/Ep2Y+9IRJzXvF6PmPupnUClrEs8PO8KA=";
            aarch64-linux = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
          };

          mkDesktopNodeModules =
            {
              pname,
              version,
              src,
              outputHash,
            }:
            pkgs.stdenv.mkDerivation {
              inherit pname version src;

              nativeBuildInputs = with pkgs; [ bun ];

              # stdenv's automatic patchShebangs would rewrite vendored
              # scripts' shebangs to the store's bash, embedding store paths
              # in a fixed-output tree (which must stay path-free). Nothing
              # here execs those scripts — mkDesktopFrontend runs Vite
              # through `node` directly — so leave them alone.
              dontPatchShebangs = true;

              buildPhase = ''
                runHook preBuild
                export HOME=$TMPDIR
                # bun 1.4's default linker is `isolated`: $out would be a
                # `.bun` content store with no resolvable tree for Node.
                # `--linker=hoisted` writes real directories (hardlinks out
                # of the store), so `mv` yields a closed node_modules whose
                # packages Node resolves by walking up — which is exactly
                # what Vite finds from crates/outl-desktop and what the
                # `node .../vite/bin/vite.js` invocation below loads.
                bun install --frozen-lockfile --linker=hoisted
                # Workspace entries (the clients, plugin-sdk, examples) are
                # relative links into the source tree; `mv` below orphans
                # them. Replace each with a real copy so the tree is closed
                # and fixupPhase's symlink audit passes.
                for l in node_modules/outl* node_modules/@outl/*; do
                  if [ -L "$l" ]; then
                    t="$(readlink -f "$l")"
                    rm "$l"
                    cp -rL "$t" "$l"
                  fi
                done
                mv node_modules $out
                runHook postBuild
              '';

              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
              inherit outputHash;
            };

          mkDesktopFrontend =
            {
              pname,
              version,
              src,
              deps,
            }:
            pkgs.stdenv.mkDerivation {
              inherit pname version src;

              nativeBuildInputs = with pkgs; [ nodejs ];

              buildPhase = ''
                runHook preBuild
                # Vendored deps at the workspace root, beside the unpacked
                # source: Node walks up from crates/outl-desktop to find
                # them, which is how vite.config.ts's plugin imports resolve.
                # Writable copy: Vite's dep optimizer caches in node_modules.
                cp -r ${deps} node_modules
                chmod -R u+w node_modules
                cd crates/outl-desktop
                # `node` provides the interpreter; the .bin shims are not
                # exec'd, so the sandbox's missing /usr/bin/env is moot.
                node ../../node_modules/vite/bin/vite.js build
                runHook postBuild
              '';

              installPhase = ''
                runHook preInstall
                mkdir -p $out
                cp -r dist/* $out/
                runHook postInstall
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

          desktopNodeModules = mkDesktopNodeModules {
            pname = "outl-desktop-node-modules";
            inherit version;
            src = projectSrc;
            outputHash = frontendNodeModulesHash.${system};
          };

          desktopFrontend = mkDesktopFrontend {
            pname = "outl-desktop-frontend";
            inherit version;
            src = projectSrc;
            deps = desktopNodeModules;
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
            outl-desktop-node-modules = desktopNodeModules;
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
