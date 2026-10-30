//! Tests for the overlay renderers.
//!
//! Split out of `overlays.rs` to keep that file under the size ratchet
//! (`scripts/check-file-size.sh`); the layout follows
//! `view/outline.rs` + `view/outline/tests.rs`.

#[cfg(test)]
mod help_coverage_tests {
    //! The help popup is a hand-curated list, not generated from
    //! `outl-shortcuts`. That buys a layout worded for the terminal
    //! and costs the guarantee that a new chord shows up on its own:
    //! `g r` / `g R` / `g n` / `g s` shipped working and undiscoverable
    //! because nobody edited this file.
    //!
    //! These pin the reminder rows. The chord *spelling* is guarded
    //! separately against the catalog in `input/normal.rs`, so between
    //! the two a re-spelled or a dropped chord fails a test.

    use crate::theme::default_theme;
    use crate::view::overlays::help_tab_body;

    fn normal_help() -> String {
        let theme = default_theme();
        help_tab_body(0, &theme, &crate::icons::IconSet::default())
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_reminder_chord_is_listed() {
        let help = normal_help();
        for chord in ["g r", "g R", "g n", "g s"] {
            assert!(
                help.contains(chord),
                "`{chord}` works but isn't in the help; the popup is \
                 hand-written, so adding a chord means editing it too"
            );
        }
    }

    #[test]
    fn the_property_editor_and_its_keys_are_listed() {
        // `g p` used to mean "pin"; the re-spelling only pays off if
        // both the new chord and the moved one are findable here.
        // And an overlay whose inner keys aren't documented is a
        // list the user can open and not operate.
        let help = normal_help();
        for chord in ["g p", "g P", "dd delete", "Tab completes"] {
            assert!(
                help.contains(chord),
                "`{chord}` works but isn't in the help; the popup is \
                 hand-written, so adding a chord means editing it too"
            );
        }
    }

    #[test]
    fn the_help_says_how_to_edit_a_rule() {
        // `g r` writes a starter rule the user almost always tunes.
        // Without this line the only way to find `:prop` is the toast
        // that scrolls away.
        assert!(normal_help().contains(":prop remind"));
    }

    #[test]
    fn the_first_tab_is_the_one_these_chords_live_on() {
        // `normal_help()` hardcodes tab 0; if the tab order ever
        // changes these assertions would silently test the wrong list.
        assert_eq!(crate::view::HELP_TABS.first().copied(), Some("Normal"));
    }
}

#[cfg(test)]
mod scroll_tests {
    use crate::view::overlays::scroll_to_keep_visible;

    #[test]
    fn a_selection_already_on_screen_does_not_scroll() {
        assert_eq!(scroll_to_keep_visible(0, 10), 0);
        assert_eq!(scroll_to_keep_visible(9, 10), 0);
    }

    #[test]
    fn a_selection_past_the_viewport_scrolls_just_enough() {
        assert_eq!(scroll_to_keep_visible(10, 10), 1);
        assert_eq!(scroll_to_keep_visible(14, 10), 5);
    }

    #[test]
    fn a_tall_viewport_does_not_scroll_early() {
        // The old `selected > 5` rule scrolled here while 20 rows were
        // on screen and nothing needed moving.
        assert_eq!(scroll_to_keep_visible(7, 20), 0);
    }

    #[test]
    fn a_short_viewport_scrolls_enough() {
        // And it under-scrolled here: row 7 in a 3-row card needs an
        // offset of 5, not 2.
        assert_eq!(scroll_to_keep_visible(7, 3), 5);
    }

    #[test]
    fn a_zero_height_viewport_returns_zero_instead_of_underflowing() {
        assert_eq!(scroll_to_keep_visible(4, 0), 0);
    }
}
