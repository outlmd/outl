use super::*;

#[test]
fn defaults_match_documented_values() {
    let c = Config::default();
    assert_eq!(c.theme.preset, "outl-light");
    assert_eq!(c.theme.dark(), "outl");
    assert!(c.editor.vim_mode);
    assert_eq!(c.editor.font_size, 15);
    assert_eq!(c.editor.diagrams, DiagramsMode::Pele);
    assert!(c.workspace.last.is_none());
    assert!(c.calendar.timezone.is_none());
    assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
    assert!(c.sync.relay_url.is_none());
    assert!(c.snapshot.enabled);
    assert_eq!(c.snapshot.op_threshold, 10_000);
    assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
    assert!(c.display.backlinks_order.newest_first());
    assert!(c.backup.enabled, "backups default ON — see BackupCfg docs");
    assert_eq!(c.backup.interval_minutes, 30);
}

/// The partial-TOML path required when adding a section: a config
/// that predates `[backup]` must still get the defaults.
#[test]
fn backup_section_defaults_when_absent() {
    let c: Config = toml::from_str("[theme]\npreset = \"outl\"\n").unwrap();
    assert!(c.backup.enabled);
    assert_eq!(c.backup.interval_minutes, 30);
}

#[test]
fn backup_section_parses() {
    let c: Config = toml::from_str("[backup]\nenabled = false\ninterval_minutes = 120\n").unwrap();
    assert!(!c.backup.enabled);
    assert_eq!(c.backup.interval_minutes, 120);
}

#[test]
fn missing_diagrams_stays_pele() {
    let c: Config = toml::from_str("[editor]\nvim_mode = false\n").unwrap();
    assert_eq!(c.editor.diagrams, DiagramsMode::Pele);
    assert!(!c.editor.vim_mode);
}

#[test]
fn diagrams_off_parses() {
    let c: Config = toml::from_str("[editor]\ndiagrams = \"off\"\n").unwrap();
    assert_eq!(c.editor.diagrams, DiagramsMode::Off);
    assert!(c.editor.vim_mode);
    assert_eq!(c.editor.font_size, 15);
}

#[test]
fn display_section_parses_backlinks_order() {
    let c: Config = toml::from_str("[display]\nbacklinks_order = \"oldest\"\n").unwrap();
    assert_eq!(c.display.backlinks_order, BacklinksOrder::Oldest);
    assert!(!c.display.backlinks_order.newest_first());
}

#[test]
fn missing_display_section_defaults_to_newest() {
    // Only [theme] populated → display falls back to its default
    // (newest-first), so an older config keeps the issue-#142 fix.
    let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
    assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
}

#[test]
fn partial_assets_section_keeps_other_defaults() {
    // A config with ONLY [assets] populated must leave every other
    // section at its default (the schema-change checklist).
    let c: Config = toml::from_str("[assets]\nmax_bytes = 5000\n").unwrap();
    assert_eq!(c.assets.max_bytes, 5000);
    assert_eq!(c.theme.preset, "outl-light");
    assert_eq!(c.theme.dark(), "outl");
    assert!(c.editor.vim_mode);
    assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
    assert_eq!(c.display.backlinks_order, BacklinksOrder::Newest);
}

#[test]
fn missing_assets_section_defaults_to_100_mib() {
    let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
    assert_eq!(c.assets.max_bytes, 100 * 1024 * 1024);
}

#[test]
fn calendar_section_parses_timezone() {
    let c: Config = toml::from_str("[calendar]\ntimezone = \"Europe/London\"\n").unwrap();
    assert_eq!(c.calendar.timezone.as_deref(), Some("Europe/London"));
}

#[test]
fn missing_calendar_section_leaves_timezone_unset() {
    // No [calendar] → timezone None → clock uses OS local (previous behaviour).
    let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
    assert!(c.calendar.timezone.is_none());
}

#[test]
fn empty_config_defaults_to_iroh_transport() {
    // P2P is outl's primary sync, so a missing [sync] section defaults to
    // iroh. `transport = "file"` is the explicit opt-out.
    let c: Config = toml::from_str("").unwrap();
    assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
    assert!(c.sync.relay_url().is_none());
}

#[test]
fn sync_section_parses_file_transport() {
    let c: Config = toml::from_str("[sync]\ntransport = \"file\"\n").unwrap();
    assert_eq!(c.sync.transport, SyncTransportKind::File);
}

#[test]
fn sync_section_parses_iroh_transport() {
    let c: Config = toml::from_str(
        r#"
[sync]
transport = "iroh"
"#,
    )
    .unwrap();
    assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
    // No relay set → falls back to defaults (None).
    assert!(c.sync.relay_url().is_none());
}

#[test]
fn sync_empty_relay_url_normalizes_to_none() {
    let c: Config = toml::from_str(
        r#"
[sync]
transport = "iroh"
relay_url = ""
"#,
    )
    .unwrap();
    assert!(c.sync.relay_url().is_none());
}

#[test]
fn sync_relay_url_is_returned_when_set() {
    let c: Config = toml::from_str(
        r#"
[sync]
transport = "iroh"
relay_url = "https://relay.example"
"#,
    )
    .unwrap();
    assert_eq!(c.sync.relay_url(), Some("https://relay.example"));
}

#[test]
fn missing_snapshot_section_uses_defaults() {
    // No [snapshot] → enabled=true, op_threshold=10_000 (the
    // long-lived clients' default). A user who never heard of
    // snapshots still gets the boot speedup.
    let c: Config = toml::from_str("").unwrap();
    assert!(c.snapshot.enabled);
    assert_eq!(c.snapshot.op_threshold, 10_000);
}

#[test]
fn snapshot_section_can_disable_writes() {
    // The CLI overrides this in-memory regardless, but a user can
    // also opt out globally by writing `enabled = false`.
    let c: Config = toml::from_str("[snapshot]\nenabled = false\n").unwrap();
    assert!(!c.snapshot.enabled);
    // op_threshold keeps its default.
    assert_eq!(c.snapshot.op_threshold, 10_000);
}

#[test]
fn snapshot_section_can_tune_threshold() {
    let c: Config = toml::from_str("[snapshot]\nop_threshold = 1000\n").unwrap();
    assert_eq!(c.snapshot.op_threshold, 1000);
    // enabled keeps its default.
    assert!(c.snapshot.enabled);
}

#[test]
fn reminders_deliver_by_default() {
    // `remind::` on a block IS the opt-in. A device that never
    // gets a rule never fires, so defaulting off only bought the
    // user a rule that silently did nothing.
    let c: Config = toml::from_str("").unwrap();
    assert!(c.reminders.enabled);
    assert_eq!(c.reminders.quiet_window(), None);
}

#[test]
fn reminders_can_still_be_switched_off() {
    let c: Config = toml::from_str("[reminders]\nenabled = false\n").unwrap();
    assert!(!c.reminders.enabled);
}

#[test]
fn quiet_hours_parses_a_wrapping_window() {
    let c: Config =
        toml::from_str("[reminders]\nenabled = true\nquiet_hours = \"22:00-07:00\"\n").unwrap();
    assert!(c.reminders.enabled);
    assert_eq!(c.reminders.quiet_window(), Some((22 * 60, 7 * 60)));
}

#[test]
fn unparseable_quiet_hours_is_ignored_not_fatal() {
    // A typo here must never keep the app from opening.
    for bad in ["22:00", "banana", "25:00-07:00", "22:00-22:00", ""] {
        let c: Config = toml::from_str(&format!("[reminders]\nquiet_hours = \"{bad}\"\n")).unwrap();
        assert_eq!(c.reminders.quiet_window(), None, "{bad} should not parse");
    }
}

#[test]
fn a_config_with_only_preset_behaves_exactly_as_before() {
    // RFC 0022's whole backwards-compatibility story. `mode`
    // defaults to auto, but with no `preset_dark` both sides of
    // the pair are the same preset, so auto alternates between
    // dracula and dracula — today's behaviour, byte for byte.
    // If someone "simplifies" the None fallback away, every
    // pre-RFC config silently starts theme-switching.
    let c: Config = toml::from_str("[theme]\npreset = \"dracula\"\n").unwrap();
    assert_eq!(c.theme.preset, "dracula");
    assert_eq!(c.theme.dark(), "dracula");
    assert_eq!(c.theme.mode, ThemeMode::Auto);
}

#[test]
fn a_pair_keeps_the_two_sides_apart() {
    let c: Config = toml::from_str(
        "[theme]\nmode = \"auto\"\npreset = \"logseq-light\"\npreset_dark = \"outl\"\n",
    )
    .unwrap();
    assert_eq!(c.theme.preset, "logseq-light");
    assert_eq!(c.theme.dark(), "outl");
    assert_eq!(c.theme.mode, ThemeMode::Auto);
}

#[test]
fn mode_round_trips_through_toml() {
    for (text, want) in [
        ("light", ThemeMode::Light),
        ("dark", ThemeMode::Dark),
        ("auto", ThemeMode::Auto),
    ] {
        let c: Config = toml::from_str(&format!("[theme]\nmode = \"{text}\"\n")).unwrap();
        assert_eq!(c.theme.mode, want);
        let back = toml::to_string(&c).unwrap();
        assert!(back.contains(&format!("mode = \"{text}\"")), "{back}");
    }
}
