flake:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.outl;

  tomlFormat = pkgs.formats.toml { };

  themePresets = [
    "outl"
    "outl-light"
    "default-dark"
    "light"
    "logseq-light"
    "dracula"
    "solarized-dark"
    "nord"
    "monokai"
    "gruvbox"
  ];
in
{
  options.programs.outl = {
    enable = lib.mkEnableOption "outl outliner";

    package = lib.mkOption {
      type = lib.types.package;
      default = flake.packages.${pkgs.system}.outl;
      defaultText = lib.literalMD "the upstream `outl` package from this flake";
      description = ''
        The outl package to use. The flake default builds the CLI and TUI
        binaries from the sources in this flake.
      '';
    };

    desktopPackage = lib.mkOption {
      type = lib.types.package;
      default = flake.packages.${pkgs.system}.outl-desktop;
      defaultText = lib.literalMD "the upstream `outl-desktop` package from this flake";
      description = ''
        The outl-desktop package to use. The flake default builds it from the
        sources in this flake. The freedesktop integration under
        `installDesktop` expects the outl-desktop layout (a
        `$out/share/applications/<id>.desktop` entry and matching hicolor icons,
        where `<id>` is `desktopId`); a substitute that does not emit those
        paths yields dangling symlinks rather than an error.
      '';
    };

    installDesktop = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether to also install the outl-desktop Tauri application.";
    };

    desktopId = lib.mkOption {
      type = lib.types.str;
      default = "app.outl.desktop";
      description = ''
        The freedesktop application id behind the launcher entry and icon that
        `installDesktop` installs into `~/.local/share`. It must match the `$ID`
        the `desktopPackage` writes under `$out/share` (its postInstall derives
        it from `tauri.conf.json`'s `identifier`) — the symlinked `.desktop` and
        icon basenames are built from this value, so a mismatch silently
        produces dangling links instead of an icon. The default is the upstream
        `outl-desktop` identifier; change it only when `desktopPackage` points at
        a build whose identifier differs.
      '';
    };

    services.sync = {
      enable = lib.mkEnableOption "outl background sync service (Linux only)";

      workspace = lib.mkOption {
        type = lib.types.str;
        description = "Path to the workspace to sync.";
      };

      watch = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Run the file watcher (reconciles .md written into the workspace from outside into the op log). Disable for a pure endpoint holder: cheaper to leave running beside a GUI or TUI, since it takes no per-actor write lock.";
      };

      sync = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Hold this device's iroh endpoint so paired peers converge continuously.";
      };

      rustLog = lib.mkOption {
        type = lib.types.str;
        default = "info";
        example = "outl=debug,iroh=info";
        description = "RUST_LOG for the daemon. Use outl=debug when diagnosing a pairing that will not connect.";
      };
    };

    settings = lib.mkOption {
      type = lib.types.submodule {
        options = {
          managed = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = lib.mdDoc ''
              Whether home-manager owns `~/.config/outl/config.toml` outright.

              When true (the default) the generated config carries `managed =
              true`, and every client — outl, outl-desktop, outl-tui, mobile —
              refuses to rewrite the file. The setting is written by Nix, not by
              the app, so the managed symlink is never detached by a client
              persisting `workspace.last`, the theme toggle or the backlinks
              direction. That rewrite is exactly what made a later
              `home-manager switch` abort with "file exists and cannot be
              overridden".

              Set to false to let the clients persist their own runtime state
              again — accepting that a client write then detaches home-manager's
              symlink, and the next activation only reclaims it if you also set
              symlink, and a later activation refuses a conflicting file until you move it aside or configure Home Manager's backup handling.

              When true, pin `settings.workspace.last` if you want the desktop to
              reopen your last workspace: the app can no longer store it.
            '';
          };

          workspace = {
            last = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Absolute path to the last workspace opened.";
            };
          };

          theme = {
            preset = lib.mkOption {
              type = lib.types.enum themePresets;
              default = "outl-light";
              description = lib.mdDoc ''
                Theme palette preset — the light side of the light/dark pair.
                Names match outl_theme::PRESETS.

                Two caveats on where this lands:

                - This module writes the **global** `~/.config/outl/config.toml`,
                  which is outl's *lowest* theme precedence. A per-workspace
                  `<root>/.outl/config.toml` `[theme] preset` and the `--theme`
                  flag both override it wholesale. For this setting to take
                  effect in a workspace, that workspace's `.outl/config.toml`
                  must carry **no** `[theme]` section.
                - The TUI always renders the **dark** side (`presetDark`) under
                  `mode = "auto"` or `"dark"` — a terminal cannot read OS
                  appearance. Since `presetDark` defaults to following
                  `preset`, setting this option alone *does* theme the terminal.
                  Set `presetDark` explicitly only when you want the desktop's
                  light and dark sides to differ.
              '';
            };

            presetDark = lib.mkOption {
              type = lib.types.nullOr (lib.types.enum themePresets);
              default = null;
              defaultText = lib.literalExpression "null  # follows preset";
              description = lib.mdDoc ''
                Dark side of the pair — the preset the TUI actually renders
                under `mode = "auto"` or `"dark"`. `null` (the default) makes
                it follow `preset`, so a single `theme.preset` themes both the
                terminal and the desktop. Set it explicitly only to make the
                light and dark sides differ (for example a custom `preset` with
                the brand `"outl"` dark theme). When nothing is customised the
                module emits the brand pair `outl-light` / `outl`, so the
                terminal keeps the brand dark theme rather than defaulting to
                the light preset.
              '';
            };

            mode = lib.mkOption {
              type = lib.types.enum [
                "light"
                "dark"
                "auto"
              ];
              default = "auto";
              description = lib.mdDoc ''
                Which side of the pair to render. The TUI cannot read OS
                appearance and treats `"auto"` as **dark** (renders
                `presetDark`), so `"auto"` is a desktop setting as far as the
                terminal is concerned.
              '';
            };
          };

          editor = {
            vimMode = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Enable vim-style modal bindings in the desktop client.";
            };

            fontSize = lib.mkOption {
              type = lib.types.ints.between 0 4294967295;
              default = 15;
              description = "Outline font size in pixels (desktop only).";
            };
          };

          calendar = {
            timezone = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "IANA timezone name (e.g. 'Europe/London'). Uses OS timezone when unset.";
            };
          };

          sync = {
            transport = lib.mkOption {
              type = lib.types.enum [
                "iroh"
                "file"
              ];
              default = "iroh";
              description = "Sync transport: 'iroh' for P2P QUIC, 'file' for iCloud/shared FS.";
            };

            relayUrl = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Custom iroh relay URL. Uses outl's default relay when unset.";
            };
          };

          display = {
            backlinksOrder = lib.mkOption {
              type = lib.types.enum [
                "newest"
                "oldest"
              ];
              default = "newest";
              description = "Sort direction for backlinks list.";
            };
          };

          assets = {
            maxBytes = lib.mkOption {
              type = lib.types.ints.between 0 9223372036854775807;
              default = 104857600;
              description = "Maximum size in bytes for a single uploaded file. 0 = unbounded.";
            };
          };

          reminders = {
            enabled = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Whether this device delivers reminder notifications.";
            };

            quietHours = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "22:00-07:00";
              description = "Time window where reminders are deferred (e.g. '22:00-07:00').";
            };
          };

          snapshot = {
            enabled = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Enable materialized-state snapshots for faster boot.";
            };

            opThreshold = lib.mkOption {
              type = lib.types.ints.between 0 4294967295;
              default = 10000;
              description = "Number of ops between snapshot writes.";
            };
          };

          storage = {
            lruCap = lib.mkOption {
              type = lib.types.ints.between 0 9223372036854775807;
              default = 20000;
              description = "Maximum ops held in memory. 0 = unbounded.";
            };
          };

          tui = {
            icons = lib.mkOption {
              type = lib.types.enum [
                "emoji"
                "nerd-font"
              ];
              default = "emoji";
              description = ''
                Chrome icon set for the TUI. "emoji" (default) uses unicode
                glyphs that work with ordinary terminal fonts; "nerd-font" opts
                into compact Font Awesome / Material Design Nerd Font glyphs and
                needs a patched font installed or those cells render as tofu.
              '';
            };

            mouseCapture = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Capture mouse events in TUI (disables terminal text selection).";
            };
          };

          backup = {
            enabled = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Enable automatic git snapshots of the workspace.";
            };

            intervalMinutes = lib.mkOption {
              type = lib.types.ints.between 0 9223372036854775807;
              default = 30;
              description = "Minimum minutes between automatic snapshots.";
            };
          };

          extraConfig = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
            example = lib.literalExpression ''
              {
                custom_section = {
                  key = "value";
                };
              }
            '';
            description = "Additional configuration to merge into the generated config.toml. Use for fields not yet modeled by this module.";
          };
        };
      };
      default = { };
      description = "outl configuration. See https://outl.app/docs/config";
    };
  };

  config = lib.mkIf cfg.enable (
    let
      # The dark side of the theme pair follows the light side unless the user
      # overrides `presetDark`, so a single `theme.preset` themes both the
      # terminal (which renders the dark side under mode auto|dark) and the
      # desktop. When neither is customised we keep the brand pair
      # `outl-light` / `outl`: a `[theme]` section carrying only `preset` would
      # otherwise make `dark()` resolve to the light preset and render the
      # terminal in light mode. Docs: docs/nix.md → "Theme precedence".
      tc = cfg.settings.theme;
      presetDarkValue =
        if tc.presetDark != null then
          tc.presetDark
        else if tc.preset != "outl-light" then
          tc.preset
        else
          "outl";

      configData =
        let
          s = cfg.settings;
        in
        lib.recursiveUpdate ({
          # Top-level directive, not a section: tells every client not to
          # rewrite the file (see the `managed` option). `pkgs.formats.toml`
          # emits scalar keys before any `[table]`, so this stays a top-level
          # key rather than landing inside the first section.
          managed = s.managed;

          workspace = lib.filterAttrs (_: v: v != null) {
            last = s.workspace.last;
          };

          theme = {
            preset = s.theme.preset;
            mode = s.theme.mode;
            preset_dark = presetDarkValue;
          };

          editor = {
            vim_mode = s.editor.vimMode;
            font_size = s.editor.fontSize;
          };

          calendar = lib.filterAttrs (_: v: v != null) {
            timezone = s.calendar.timezone;
          };

          sync = lib.filterAttrs (_: v: v != null) {
            transport = s.sync.transport;
            relay_url = s.sync.relayUrl;
          };

          display = {
            backlinks_order = s.display.backlinksOrder;
          };

          assets = {
            max_bytes = s.assets.maxBytes;
          };

          reminders = {
            enabled = s.reminders.enabled;
          }
          // lib.filterAttrs (_: v: v != null) {
            quiet_hours = s.reminders.quietHours;
          };

          snapshot = {
            enabled = s.snapshot.enabled;
            op_threshold = s.snapshot.opThreshold;
          };

          storage = {
            lru_cap = s.storage.lruCap;
          };

          tui = {
            icons = s.tui.icons;
            mouse_capture = s.tui.mouseCapture;
          };

          backup = {
            enabled = s.backup.enabled;
            interval_minutes = s.backup.intervalMinutes;
          };
        }) s.extraConfig;

      configFile = tomlFormat.generate "outl-config" configData;
    in
    {
      # Packages are only built for Linux (see flake.nix). On other platforms
      # we still generate the config file below, but install nothing — users
      # there provide outl themselves (e.g. the official installer).
      home.packages = lib.optionals pkgs.stdenv.isLinux (
        [
          cfg.package
        ]
        ++ lib.optional cfg.installDesktop cfg.desktopPackage
      );

      # Keep Home Manager's collision guard enabled. The managed directive
      # prevents outl clients from replacing the generated symlink after
      # activation; first-time conflicts must be resolved with Home Manager's
      # backup policy or by moving the old file aside.
      xdg.configFile."outl/config.toml".source = configFile;

      # Desktop integration for the Tauri GUI, gated on `installDesktop`.
      #
      # `mkOutlDesktop` (flake.nix) already emits a validated freedesktop entry
      # and hicolor icons into the package's `$out/share`, and home-manager
      # deep-links `/share` into the profile. That is enough for a full
      # desktop session, but a bare Wayland launcher (e.g. `rofi -show drun`
      # under a tiling WM) resolves the `.desktop` entry from the profile yet
      # can miss the *themed* icon it references — the icon lives only in the
      # store, off every path the launcher actually scans. Drop the entry and
      # its icons into the user data dir so every launcher finds them. The
      # sources are symlinks into the package, so it stays the single owner.
      #
      # `dataFile` is generated regardless of `xdg.enable`, and `dataHome`
      # defaults to `~/.local/share`, which is always on `XDG_DATA_DIRS`.
      xdg.dataFile = lib.optionalAttrs (cfg.installDesktop && pkgs.stdenv.hostPlatform.isLinux) (
        let
          # Basenames the desktopPackage emits under $out/share; keep in sync
          # via the `desktopId` option rather than a second hardcoded copy.
          id = cfg.desktopId;
          share = "${cfg.desktopPackage}/share";
        in
        {
          "applications/${id}.desktop".source = "${share}/applications/${id}.desktop";
          # Brand PNG sizes only — no `hicolor/scalable`, mirroring the package
          # (Tauri's `icon.svg` is a placeholder glyph, so `mkOutlDesktop` ships
          # no scalable entry; linking one here would dangle and shadow the PNGs).
          "icons/hicolor/32x32/apps/${id}.png".source = "${share}/icons/hicolor/32x32/apps/${id}.png";
          "icons/hicolor/128x128/apps/${id}.png".source = "${share}/icons/hicolor/128x128/apps/${id}.png";
          "icons/hicolor/256x256/apps/${id}.png".source = "${share}/icons/hicolor/256x256/apps/${id}.png";
        }
      );

      # Both combinations below make `outl serve` exit non-zero, which under
      # Restart=on-failure becomes a crash loop — refuse them at eval time.
      # The { assertion, message } shape is what home-manager's check pass
      # expects; a bare bool here breaks every evaluation of this module.
      assertions = lib.optionals cfg.services.sync.enable [
        {
          assertion = pkgs.stdenv.hostPlatform.isLinux;
          message = "programs.outl.services.sync is supported only on Linux";
        }
        {
          assertion = cfg.services.sync.watch || cfg.settings.sync.transport != "file";
          message = "programs.outl.services.sync.watch = false with settings.sync.transport = \"file\" has nothing left to do: outl serve refuses that combination";
        }
        {
          assertion = cfg.services.sync.watch || cfg.services.sync.sync;
          message = "programs.outl.services.sync: watch = false and sync = false is a usage error for outl serve";
        }
      ];

      systemd.user.services.outl-sync =
        lib.mkIf (cfg.services.sync.enable && pkgs.stdenv.hostPlatform.isLinux)
          {
            Unit = {
              Description = "outl background sync service";
              After = [ "network-online.target" ];
              Wants = [ "network-online.target" ];
            };

            Service = {
              Environment = [ "RUST_LOG=${cfg.services.sync.rustLog}" ];
              ExecStart =
                "${cfg.package}/bin/outl serve --workspace ${lib.escapeShellArg cfg.services.sync.workspace}"
                + lib.optionalString (!cfg.services.sync.watch) " --no-watch"
                + lib.optionalString (!cfg.services.sync.sync) " --no-sync";
              Restart = "on-failure";
              RestartSec = "5s";
            };

            Install = {
              WantedBy = [ "default.target" ];
            };
          };
    }
  );
}
