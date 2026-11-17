//! The outbound links outl puts in front of the user, declared once.
//!
//! Two surfaces show them: the macOS Help menu ([`crate::menu`]) and
//! the About window ([`crate::about`], rendered by the frontend's
//! `AboutWindow.tsx`). A label or a URL spelled in both places is two
//! owners of one fact, which is how a "Report a problem" item ends up
//! pointing at a repository that moved (root `CLAUDE.md` → One owner
//! per fact).
//!
//! So the table lives here, the menu builds itself from it, the click
//! router resolves an id back through it, and the frontend mirror
//! (`src/lib/links.ts`) is pinned to this file by
//! `the_frontend_mirror_matches_this_table`. The mirror is hand-written
//! on purpose — there is no bindgen step on this frontend — and the
//! pin is what makes that safe, the same shape `outl-tauri-shared`'s
//! `wire_mirrors.rs` uses for the DTOs.
//!
//! **The repository URL is spelled `outlmd/outl`.** GitHub redirects
//! the old personal-profile path, so a stale link still lands; it also
//! shows the user a URL that is not where the project lives.

/// One link: the id the menu item carries, the label both surfaces
/// show, and where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link {
    /// Menu item id. Also the key the frontend mirror lists it under.
    pub id: &'static str,
    /// What the user reads. Title Case, because these are macOS menu
    /// items first and the About window reuses them rather than
    /// writing a second wording.
    pub label: &'static str,
    /// Where it opens, in the user's browser.
    pub url: &'static str,
}

/// The source, for someone who wants to read it or build it.
pub const REPOSITORY: Link = Link {
    id: "help-repository",
    label: "outl on GitHub",
    url: "https://github.com/outlmd/outl",
};

/// Where a bug report or a feature request goes.
pub const ISSUES: Link = Link {
    id: "help-issues",
    label: "Report a Problem",
    url: "https://github.com/outlmd/outl/issues",
};

/// GitHub Sponsors. Points at the maintainer's profile, not at the
/// org: that is where the sponsorship tiers are configured.
pub const SPONSOR: Link = Link {
    id: "help-sponsor",
    label: "Sponsor outl",
    url: "https://github.com/sponsors/avelino",
};

/// The Help menu, in the order it is drawn — and the order the About
/// window lists them in, for the same reason the labels are shared.
pub const HELP_LINKS: &[Link] = &[REPOSITORY, ISSUES, SPONSOR];

/// The link a menu item id belongs to, if any.
pub fn by_id(id: &str) -> Option<&'static Link> {
    HELP_LINKS.iter().find(|link| link.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_link_resolves_from_its_own_id() {
        for link in HELP_LINKS {
            assert_eq!(by_id(link.id), Some(link));
        }
        assert_eq!(by_id("quit"), None);
        assert_eq!(by_id(""), None);
    }

    #[test]
    fn ids_and_urls_are_unique() {
        for (i, a) in HELP_LINKS.iter().enumerate() {
            for b in &HELP_LINKS[i + 1..] {
                assert_ne!(a.id, b.id, "two links share the id {:?}", a.id);
                assert_ne!(a.url, b.url, "two links share the url {:?}", a.url);
            }
        }
    }

    /// A menu that opens `http://` hands the user's browser a page an
    /// network between here and GitHub can rewrite. The capability
    /// file allows `https://*` for the webview side; this pins the
    /// backend side, which the ACL never sees.
    #[test]
    fn every_link_is_https_and_labelled() {
        for link in HELP_LINKS {
            assert!(
                link.url.starts_with("https://"),
                "{:?} is not https: {:?}",
                link.id,
                link.url,
            );
            assert!(
                link.label.len() > 4,
                "{:?} has a label too terse to read: {:?}",
                link.id,
                link.label,
            );
        }
    }

    /// The frontend mirror is hand-written; this is what keeps it
    /// honest. It reads the file rather than importing it, because
    /// Rust cannot parse TypeScript — a label changed on one side and
    /// not the other fails here, which is the whole point.
    #[test]
    fn the_frontend_mirror_matches_this_table() {
        let mirror = include_str!("../../src/lib/links.ts");
        for link in HELP_LINKS {
            assert!(
                mirror.contains(link.id),
                "src/lib/links.ts is missing the id {:?}",
                link.id,
            );
            assert!(
                mirror.contains(link.label),
                "src/lib/links.ts is missing the label {:?} — the menu and the About \
                 window must read the same",
                link.label,
            );
            assert!(
                mirror.contains(link.url),
                "src/lib/links.ts is missing the url {:?}",
                link.url,
            );
        }
    }
}
