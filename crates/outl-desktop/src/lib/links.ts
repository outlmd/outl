/**
 * The project's outbound links, mirrored from the backend.
 *
 * `src-tauri/src/links.rs` is the owner: it builds the macOS Help menu
 * from this same table, and `the_frontend_mirror_matches_this_table`
 * fails if an id, a label or a URL here stops matching it. Edit that
 * file first, then this one.
 *
 * The mirror is hand-written because this frontend has no bindgen
 * step, the same arrangement `@outl/shared`'s DTO types have with
 * `outl-tauri-shared`.
 */

/** One link: menu id, the label both surfaces show, where it goes. */
export interface AppLink {
  id: string;
  label: string;
  url: string;
}

/**
 * In the order the Help menu draws them, which is the order the About
 * window lists them in.
 */
export const HELP_LINKS: AppLink[] = [
  {
    id: "help-repository",
    label: "outl on GitHub",
    url: "https://github.com/outlmd/outl",
  },
  {
    id: "help-issues",
    label: "Report a Problem",
    url: "https://github.com/outlmd/outl/issues",
  },
  {
    id: "help-sponsor",
    label: "Sponsor outl",
    url: "https://github.com/sponsors/avelino",
  },
];
