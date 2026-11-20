use super::empty_section_notice;

#[test]
fn a_page_with_backlinks_gets_no_notice() {
    assert_eq!(empty_section_notice(true, true, false), None);
    assert_eq!(empty_section_notice(true, true, true), None);
}

#[test]
fn a_page_with_none_and_a_settled_index_gets_no_notice() {
    // Genuinely zero backlinks: the section stays absent, which is
    // the pre-existing behaviour and the right one.
    assert_eq!(empty_section_notice(true, false, false), None);
}

#[test]
fn a_still_building_index_says_so_instead_of_nothing() {
    assert!(
        empty_section_notice(true, false, true).is_some(),
        "a pending index must be distinguishable from zero backlinks"
    );
}

#[test]
fn the_toggle_wins_over_everything() {
    // `B` off means the user asked for no section at all.
    assert_eq!(empty_section_notice(false, false, true), None);
    assert_eq!(empty_section_notice(false, true, true), None);
}
