//! The two rules that decide **what a backlink is**: which keys a
//! block's text mentions, and which keys a page looks itself up
//! under.
//!
//! They live together, apart from the index that stores them, because
//! they are two halves of one agreement — a key emitted by
//! [`mentions_of`] that [`keys_for_page`] never asks for is a mention
//! nobody can find, and the reverse is a page that finds nothing. Read
//! one, read the other.
//!
//! [`crate::backlinks_index`] owns the storage and the traversal;
//! [`crate::backlinks::backlinks_for_page`] / `backlinks_for_target`
//! are thin lookups on top of a freshly built index. None of them
//! re-derive what counts as a mention, which is the drift that once
//! made self-references visible on one client and not another.

use outl_core::workspace::Workspace;

use crate::mentions::extract_refs_and_tags;
use crate::page::PageMeta;

/// A key a block can be indexed under, mirroring the four channels the
/// old `TargetMatcher` matched on.
///
/// `Ref` is a literal `[[X]]` target (matched verbatim, like the old
/// `needle`); `Tag` is a `#tag` reduced to its slug form (so `#Avelino`
/// and page `avelino` meet); `Namespace` is an **ancestor** of a
/// namespaced mention (`#os/linux` also lands under `os`); `Call` /
/// `Provenance` are the two template channels (a ` ```call:<name> `
/// fence and a `from-template:: <slug>` property).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TargetKey {
    /// Literal `[[X]]` target.
    Ref(String),
    /// `#tag` reduced via `slugify`.
    Tag(String),
    /// A proper ancestor of a namespaced `#tag` / `[[ref]]`, reduced
    /// via `slugify` — the channel that makes `#os/linux` show up on
    /// the `os` page.
    ///
    /// Kept **separate** from `Tag` rather than folded into it, so the
    /// two questions stay answerable apart: "who tagged exactly `#os`"
    /// and "who tagged anything under `os`". Folding them would also
    /// make a namespace's own page a hit for itself the moment a
    /// deeper page mentions it.
    Namespace(String),
    /// ` ```call:<name> ` fence invocation name.
    Call(String),
    /// `from-template:: <slug>` provenance slug.
    Provenance(String),
}

/// Every key a block mentions — the single source of truth for "does
/// this block reference something".
///
/// `[[X]]` targets and `#tag`s both come out of **one** walk over the
/// inline token tree ([`extract_refs_and_tags`]), so a code span is
/// inert for both and `#avelino-foo` doesn't reduce to `avelino`; they
/// used to be read by two different rules, and one `` `code` `` span
/// then produced two opposite verdicts inside a single block. The
/// callable channel reads the fence invocation name from the text.
/// The `from-template::` provenance value is passed in by the caller —
/// the workspace build reads it off the tree, the from-disk build reads
/// it off the parsed `.md` block properties — so this one function stays
/// the sole owner of "what counts as a mention" regardless of source.
pub(crate) fn mentions_of(text: &str, from_template: Option<&str>) -> Vec<TargetKey> {
    let mut keys: Vec<TargetKey> = Vec::new();
    let (refs, tags) = extract_refs_and_tags(text);
    // Every mention also claims its namespace ancestors, from both
    // channels: `#os/linux` and `[[os/linux]]` are each a mention of
    // `os` too. Derived from the mention's **spelling**, because that
    // is where the `/` survives — the slug folds it to `-` (see
    // [`crate::namespace`]).
    let mut push = |key: TargetKey, name: &str| {
        // The flat mention is the overwhelmingly common case and has no
        // ancestors, so skip the two allocations `ancestors` would make
        // to tell us that. This runs per mention on the O(blocks) build.
        if name.contains('/') {
            for anc in crate::namespace::ancestors(name) {
                keys.push(TargetKey::Namespace(outl_md::slug::slugify(&anc)));
            }
        }
        keys.push(key);
    };
    for r in &refs {
        push(TargetKey::Ref(r.clone()), r);
    }
    for t in &tags {
        push(TargetKey::Tag(outl_md::slug::slugify(t)), t);
    }
    if let Some(name) = crate::template::call_target_name(text) {
        keys.push(TargetKey::Call(name));
    }
    if let Some(slug) = from_template {
        keys.push(TargetKey::Provenance(slug.to_string()));
    }
    keys
}

/// The keys a page looks itself up under — the lookup-side mirror of
/// [`mentions_of`], matching what `backlinks_for_page` used to scan for.
///
/// For each target string (slug, title, and the `@`-alias forms for a
/// person page) the page is found under both the literal `Ref` and the
/// `Tag` slug, exactly like the old `TargetMatcher::refs`. A template
/// page additionally looks itself up under its callable name and its
/// own slug (provenance).
pub(crate) fn keys_for_page(workspace: &Workspace, meta: &PageMeta) -> Vec<TargetKey> {
    let mut keys: Vec<TargetKey> = Vec::new();
    let mut add_target = |t: &str| {
        keys.push(TargetKey::Ref(t.to_string()));
        keys.push(TargetKey::Tag(outl_md::slug::slugify(t)));
    };
    add_target(&meta.slug);
    if meta.title != meta.slug {
        add_target(&meta.title);
    }
    if meta.page_type.as_deref() == Some(crate::person::PERSON_TYPE) {
        add_target(&format!("@{}", meta.slug));
        if meta.title != meta.slug {
            add_target(&format!("@{}", meta.title));
        }
    }
    if let Some(name) = template_name_of(workspace, meta) {
        keys.push(TargetKey::Call(name));
        keys.push(TargetKey::Provenance(meta.slug.clone()));
    }
    // A page is also the namespace root of everything nested under it:
    // `os` collects `#os/linux` and `[[os/linux/debian]]`. Looked up on
    // the page's own slug, which is the fold both sides agree on.
    keys.push(TargetKey::Namespace(meta.slug.clone()));
    if meta.title != meta.slug {
        keys.push(TargetKey::Namespace(outl_md::slug::slugify(&meta.title)));
    }
    keys
}

/// The template invocation name of `meta`'s page, when it is a template
/// (has a non-empty `template::` property).
fn template_name_of(workspace: &Workspace, meta: &PageMeta) -> Option<String> {
    let id = crate::page::find_by_slug(workspace, &meta.slug)?;
    let name = crate::page::read_text_prop(workspace, id, crate::template::TEMPLATE_KEY)?;
    (!name.trim().is_empty()).then_some(name)
}
