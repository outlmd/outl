//! The banner that says the open page has stopped syncing.
//!
//! Sits directly above the outline, in the same slot as
//! [`super::warnings_banner`], and is deliberately louder: a parser
//! warning says "outl kept something odd", this one says "this page is
//! no longer converging with your other devices, and here is what to
//! do".
//!
//! Three contracts, each of which was the alternative:
//!
//! - **Persistent, not transient.** The condition does not clear on its
//!   own — the page is frozen in both directions until
//!   `outl reconcile --ahead-of-log` runs — so a status-line message or
//!   a toast would be the same silence with a delay
//!   ([#292](https://github.com/outlmd/outl/issues/292)).
//! - **It survives focus mode.** `z f` drops the header, the sidebar,
//!   the status bar and the parse-warning banner, because all four are
//!   chrome. This is not chrome: asking for a distraction-free outline
//!   is not consent to be told nothing about a page that is not saving
//!   anywhere else.
//! - **Not dismissable.** Same reasoning as the GUI banner it mirrors
//!   (`@outl/shared`'s `<PageAheadOfLogBanner />`): a dismiss button
//!   lets a user hide a page that silently is not syncing, which is the
//!   failure this replaces.
//!
//! Every word comes from `outl_actions::refusal::ahead_of_log_notice`.
//! A second wording here would be the third copy of a sentence the
//! desktop and mobile already share.

use crate::state::App;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Banner height in lines (border included), `0` when the page is fine.
pub(crate) fn banner_height(app: &App) -> u16 {
    if app.md_ahead_of_log.is_some() {
        4
    } else {
        0
    }
}

/// Render the banner. The caller must size `area` to
/// [`banner_height`] — anything else looks broken.
pub(crate) fn render_banner(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Some(notice) = app.md_ahead_of_log.as_ref() else {
        return;
    };
    if area.height == 0 {
        return;
    }
    let accent = app.theme.destructive;
    let lines = vec![
        Line::from(Span::styled(notice.body.clone(), accent)),
        Line::from(vec![
            Span::styled(format!("{} ", notice.action), app.theme.dim),
            Span::styled(
                notice.command.to_string(),
                accent.add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    let title = format!(" {} {} ", app.icons.warning, notice.title);
    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(accent)
            .title(Span::styled(title, accent.add_modifier(Modifier::BOLD))),
    );
    f.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use crate::state::App;
    use outl_core::{ActorId, Workspace};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tempfile::TempDir;

    fn app_with_notice() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let mut app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        app.md_ahead_of_log = Some(outl_actions::refusal::ahead_of_log_notice(
            outl_actions::refusal::Surface::Tui,
            2,
            "\"only ever on disk\"",
        ));
        (app, dir)
    }

    fn painted(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| crate::view::render_app(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Everything a user needs in order to act has to be on screen: the
    /// headline, and the command. A banner that only says "something is
    /// wrong" is the same dead end as no banner.
    #[test]
    fn the_banner_names_the_condition_and_the_recovery() {
        let (mut app, _dir) = app_with_notice();
        let screen = painted(&mut app);
        assert!(
            screen.contains("This page isn't syncing"),
            "the headline must be on screen: {screen}"
        );
        assert!(
            screen.contains(outl_actions::error::AHEAD_OF_LOG_RECOVERY_COMMAND),
            "the recovery command must be on screen: {screen}"
        );
    }

    /// `z f` hides the header, the sidebar, the status bar and the
    /// parse-warning banner, because those are chrome. A page that has
    /// stopped syncing with every other device is not chrome, and
    /// wanting a distraction-free outline is not consent to be told
    /// nothing about it.
    #[test]
    fn focus_mode_does_not_hide_it() {
        let (mut app, _dir) = app_with_notice();
        app.focus_mode = true;
        let screen = painted(&mut app);
        assert!(
            screen.contains(outl_actions::error::AHEAD_OF_LOG_RECOVERY_COMMAND),
            "focus mode must not swallow the refusal: {screen}"
        );
    }

    /// The common case: nothing on screen, no row stolen from the
    /// outline.
    #[test]
    fn a_healthy_page_paints_nothing() {
        let (mut app, _dir) = app_with_notice();
        app.md_ahead_of_log = None;
        assert_eq!(super::banner_height(&app), 0);
        let screen = painted(&mut app);
        assert!(!screen.contains("isn't syncing"), "{screen}");
    }
}
