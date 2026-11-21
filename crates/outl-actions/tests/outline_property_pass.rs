//! A projection reads the property map **once**, not once per block.
//!
//! `outl_core::tree::Tree::properties_of` filters the whole
//! workspace-wide `HashMap<(NodeId, String), PropValue>` on every call.
//! Its doc comment used to promise the opposite ("without scanning the
//! whole property map"), which is how two projections of the same
//! subtree ended up on opposite sides of it; it states the cost and
//! names the pass now. `render_page_md` collapsed
//! its per-node filters into one pass long ago and recorded the win;
//! `project_outline` kept asking per node, so the DTO every GUI
//! mutation replies with cost `O(blocks × properties-in-workspace)`.
//!
//! Measured on the reference workspace (175,096 nodes, 193k properties,
//! 5,257 pages): **2,588 ms** to project one 688-block page, against
//! 10.8 ms to *render the same page to `.md`* — the same walk, through
//! the pass that already existed. A cross-page backlink commit took
//! 2.65 s end to end; the in-page `edit_block` on that page took 2.51 s,
//! because the cost was never the backlink path's, it was the reply's.
//! Nothing failed. Root `CLAUDE.md` invariant 15.
//!
//! Two of the three tests invariant 15 asks for live here, and what
//! each one drives is **exactly** this, no more:
//!
//! - *Does it still do the right thing* — equivalence against the
//!   per-node scan, over real `Op::SetProp` bodies, for each of the
//!   three callers of the pass: `project_outline` (a page),
//!   `project_outline_node` (one block's subtree), and
//!   `build_backlink_index`'s tree-side walk, which projects every
//!   citing block through `project_outline_node_shallow` off a
//!   **workspace-scoped** map. The third shipped with no case at all
//!   while this module's prose claimed every entry point was covered.
//!   Plus `the_renderer_still_drops_a_list_the_outline_joins`, the one
//!   disagreement between two of them that is deliberate.
//! - *Is it still cheap* — a token-level ratchet over the files that
//!   project a node from the tree, carrying its own list of what it
//!   cannot see. It is run against the real pre-fix `outline.rs` from
//!   `fixtures/`, because the first version of it passed there.
//!
//! Not covered here: the from-disk builders (`read_page_outline`,
//! `build_backlink_index_from_disk`) read properties out of a parse and
//! never touch the property map, so the pass is not on their path.
//!
//! The third test (*is it cheap once*) is a call-site census and lives
//! beside the commit path, in
//! `outl-tauri-shared/tests/backlink_commit_cost.rs`.

use std::path::{Path, PathBuf};

use outl_actions::{
    append_block, build_backlink_index, find_by_slug, open_or_create_page, project_outline,
    project_outline_node, render_page_md, set_property, OutlineNode, PageKind,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

/// Every `PropValue` shape, because the two projections legitimately
/// disagree about one of them: the renderer has no syntax for a `List`
/// and drops it, the outline DTO joins it. Collapsing that difference
/// while collapsing the scans would be a behaviour change wearing a
/// refactor's clothes, so the fixture has to be able to see it.
fn every_shape() -> Vec<(&'static str, PropValue)> {
    vec![
        ("zeta", PropValue::Text("last alphabetically".into())),
        ("alpha", PropValue::Text("first alphabetically".into())),
        ("owner", PropValue::PageRef("Avelino".into())),
        ("area", PropValue::Tag("infra".into())),
        (
            "people",
            PropValue::List(vec![
                PropValue::Tag("ana".into()),
                PropValue::Tag("bruno".into()),
            ]),
        ),
    ]
}

/// A page whose blocks carry properties at three depths, **plus a second
/// page carrying its own**.
///
/// The second page is the part that matters: a pass scoped to the wrong
/// subtree, or a per-node filter that leaks, shows up as a neighbour's
/// property on this page's blocks. With one page in the workspace the
/// scan and the pass cannot be told apart.
fn workspace_with_properties() -> (Workspace, HlcGenerator, NodeId, Vec<NodeId>) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");

    let other = open_or_create_page(&mut ws, &hlc, "other", "Other", PageKind::Page)
        .expect("the neighbour page is created");
    let stranger = append_block(&mut ws, &hlc, Some(other), Some("not this page's"))
        .expect("the neighbour has a block");
    for (key, value) in every_shape() {
        set_property(&mut ws, &hlc, stranger, key, Some(value)).expect("neighbour property");
    }

    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page)
        .expect("the page is created");
    // `title::` is already on the page root; add a user property so the
    // root's own entry is exercised too.
    set_property(
        &mut ws,
        &hlc,
        page,
        "role",
        Some(PropValue::Text("reference".into())),
    )
    .expect("page property");

    let top = append_block(&mut ws, &hlc, Some(page), Some("top")).expect("top block");
    let middle = append_block(&mut ws, &hlc, Some(top), Some("middle")).expect("middle block");
    let deep = append_block(&mut ws, &hlc, Some(middle), Some("deep")).expect("deep block");
    // A sibling with no properties at all: `None` and "an empty list"
    // have to stay the same answer.
    let bare = append_block(&mut ws, &hlc, Some(page), Some("bare")).expect("bare block");

    for node in [top, middle, deep] {
        for (key, value) in every_shape() {
            set_property(&mut ws, &hlc, node, key, Some(value)).expect("block property");
        }
    }

    (ws, hlc, page, vec![top, middle, deep, bare])
}

/// What `project_node` used to do per node, spelled out: the whole
/// property map, filtered to this node, mapped and sorted.
///
/// This **is** the read being replaced, so it is written out here rather
/// than called — a test that asked the new code for its own baseline
/// would pass whatever the new code does.
fn per_node_scan(workspace: &Workspace, node: NodeId) -> Vec<(String, String)> {
    fn flatten(v: &PropValue) -> String {
        match v {
            PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => s.clone(),
            PropValue::List(items) => items.iter().map(flatten).collect::<Vec<_>>().join(" "),
        }
    }
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(node)
        .map(|(k, v)| (k.to_string(), flatten(v)))
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    properties
}

fn find(nodes: &[OutlineNode], id: NodeId) -> Option<&OutlineNode> {
    let target = id.to_string();
    for node in nodes {
        if node.id == target {
            return Some(node);
        }
        if let Some(hit) = find(&node.children, id) {
            return Some(hit);
        }
    }
    None
}

/// *Does it still do the right thing* — the two page-scoped entry
/// points, against the per-node scan they replaced, over properties that
/// arrived as real `Op::SetProp` bodies.
///
/// Both branches on purpose: `project_outline` walks a page and
/// `project_outline_node` walks one block's subtree, and only the first
/// of the two was reachable from the measurement that found this.
///
/// The third caller of the pass is **not** here — its scope is the
/// workspace rather than a subtree, and it has its own case in
/// `a_backlink_carries_the_same_properties_the_per_node_scan_did`.
#[test]
fn the_projection_reads_the_same_properties_the_per_node_scan_did() {
    let (ws, _hlc, page, nodes) = workspace_with_properties();

    let outline = project_outline(&ws, page);
    for &node in &nodes {
        let projected = find(&outline, node).expect("every block is in the projection");
        assert_eq!(
            projected.properties,
            per_node_scan(&ws, node),
            "project_outline disagrees with the per-node scan for {node}"
        );
    }

    // The other branch: one block's subtree, which carries its own
    // properties *and* its descendants'.
    let top = project_outline_node(&ws, nodes[0]);
    assert_eq!(
        top.properties,
        per_node_scan(&ws, nodes[0]),
        "project_outline_node lost the root block's own properties"
    );
    for &node in &nodes[1..3] {
        let projected = find(std::slice::from_ref(&top), node)
            .expect("the subtree projection reaches its descendants");
        assert_eq!(projected.properties, per_node_scan(&ws, node));
    }

    // A block with no properties stays empty rather than inheriting a
    // neighbour's — the failure a leaking pass would produce.
    let bare = find(&outline, nodes[3]).expect("the bare block is projected");
    assert!(
        bare.properties.is_empty(),
        "a block with no properties came back with {:?}",
        bare.properties
    );

    // The scope is the subtree: the neighbour page's identically-keyed
    // properties are not on this page's blocks. With one page in the
    // workspace this assertion cannot fail, which is why the fixture has
    // two.
    assert_eq!(
        outline.len(),
        2,
        "the projection reached past this page: {outline:#?}"
    );
}

/// The one shape the two projections are **allowed** to disagree about,
/// pinned so collapsing them later is a decision somebody takes rather
/// than a side effect.
#[test]
fn the_renderer_still_drops_a_list_the_outline_joins() {
    let (ws, _hlc, page, nodes) = workspace_with_properties();

    let outline = project_outline(&ws, page);
    let top = find(&outline, nodes[0]).expect("the block is projected");
    assert_eq!(
        top.properties
            .iter()
            .find(|(k, _)| k == "people")
            .map(|(_, v)| v.as_str()),
        Some("ana bruno"),
        "the outline DTO joins a `List`"
    );

    let md = render_page_md(&ws, page);
    assert!(
        !md.contains("people::"),
        "the `.md` dialect has no syntax for a `List`, so the render drops it:\n{md}"
    );
    assert!(
        md.contains("alpha:: first alphabetically"),
        "the render still carries the shapes it does have syntax for:\n{md}"
    );
}

/// *Does it still do the right thing* — the third caller, the one the
/// ratchet found rather than the defect report.
///
/// `build_backlink_index`'s tree-side walk projects **every citing block
/// in the workspace** through `project_outline_node_shallow`, off a map
/// `outline::workspace_properties` groups once. Nothing drove it: the
/// equivalence test above reaches neither that walk nor that scope, and
/// no other suite asserts anything about a backlink's
/// `source_block.properties` — `backlinks_index_property.rs` compares
/// block *text* against a naive scan, and the cross-page case in
/// `outl-tauri-shared/tests/backlink_commit_cost.rs` compares the
/// *reply* (`project_outline`) against the parsed `.md`.
///
/// Two failures this can see that the subtree-scoped test cannot. A
/// workspace-wide `ids` set is sorted independently
/// (`workspace_properties` has its own `sort_unstable`), and the pass
/// binary-searches it, so an unsorted one drops properties silently.
/// And the value mapping is passed separately here: wiring
/// `renderable_prop_value` into this caller — the renderer's rule, one
/// line away in the same crate — would make a backlink row quietly lose
/// a `PropValue::List` the page it sits on still shows.
#[test]
fn a_backlink_carries_the_same_properties_the_per_node_scan_did() {
    let (mut ws, hlc, _page, _nodes) = workspace_with_properties();

    // A citing block on the neighbour page, carrying every shape. The
    // index projects it as a backlink of `notes`, which is the only way
    // into `project_outline_node_shallow` from outside this crate.
    let other = find_by_slug(&ws, "other").expect("the neighbour page is in the tree");
    let citing = append_block(&mut ws, &hlc, Some(other), Some("see [[notes]]"))
        .expect("the citing block is created");
    for (key, value) in every_shape() {
        set_property(&mut ws, &hlc, citing, key, Some(value)).expect("citing block property");
    }

    // `root` reaches only `page_md_path`, which joins a path; this walk
    // reads the tree, never the disk.
    let index = build_backlink_index(&ws, Path::new("/nothing-is-read-here"));
    let backlinks = index.for_target("notes");
    assert_eq!(
        backlinks.len(),
        1,
        "exactly one block mentions `notes`, so a different count means \
         this test is measuring something else: {backlinks:#?}"
    );

    let expected = per_node_scan(&ws, citing);
    assert_eq!(
        expected.len(),
        every_shape().len(),
        "the fixture has to carry properties for the comparison to mean \
         anything — two empty lists are equal"
    );
    assert_eq!(
        backlinks[0].source_block.properties, expected,
        "the backlink index's grouped pass disagrees with the per-node \
         scan it replaced"
    );
}

// ---------------------------------------------------------------------
// *Is it still cheap* — structurally, and on every branch.
// ---------------------------------------------------------------------

/// The accessor no projection may reach, as an **identifier in the
/// file's token stream** rather than as a line of text.
///
/// This started out as a lexical ratchet in `property_order_owner.rs`'s
/// shape, matching the string `"tree().properties_of("` — and the
/// defect was never written on one line. rustfmt breaks that chain
/// across three (`workspace`, `.tree()`, `.properties_of(node)`), so
/// the pattern scored **zero** against the very file it exists to
/// refuse, at both of its call sites
/// (`fixtures/outline-308d2321.rs.txt`, pinned below). A
/// multi-line-tolerant string is brittle in the other direction — it
/// has to guess which receivers and which whitespace rustfmt will
/// produce — so this hands the file to `proc-macro2` and counts
/// `properties_of` idents instead.
///
/// What that **can** see, whatever the formatting: `.properties_of(` on
/// any receiver (a `&Workspace`, a `Tree`, a local binding),
/// `Tree::properties_of(…)` as a path call, and the ident behind any
/// line break the formatter inserts. `group_properties_of` is a
/// different ident, so the pass itself cannot trip it and the receiver
/// no longer has to be part of the pattern.
///
/// What it **cannot** see, stated so nobody mistakes it for more:
///
/// - a per-node read spelled with another accessor — `tree().property(node, key)` in a loop is the same defect and passes;
/// - an ident a macro assembles, or a read in a file neither `PROJECTIONS` nor `THREADS_THE_PASS` names (the import pass's `resolve.rs` is one, deliberately out of scope);
/// - *how many times* a cheap read is asked, which is the census in `outl-tauri-shared/tests/backlink_commit_cost.rs`;
/// - whether the answer is right, which is the equivalence test above.
///
/// Rejected: `clippy.toml`'s `disallowed-methods`, which would enforce
/// this at compile time across every crate. It bans the accessor
/// workspace-wide, and ~15 call sites read one node once and are
/// correct (`text_properties_of`, the clipboard, the template
/// instantiator, `outl page rename`, `outl block prop`). Fifteen
/// `#[allow]`s, a new workspace-wide lint policy, and a forced verdict
/// on another crate's pre-existing loop is a bigger change than the
/// rule being pinned.
const PER_NODE_READ: &str = "properties_of";

/// The pass a projection must reach instead, read through the same
/// instrument.
///
/// It is the second half of what went wrong. The old assertion was
/// `src.contains("group_properties_of")`, which **a doc comment
/// satisfies** — `outline.rs` names the symbol on three comment lines —
/// so deleting both calls and keeping the prose passed a file that read
/// no properties at all. A doc comment lexes as a `#[doc = "…"]`
/// literal and a `//` comment is dropped outright, so as an ident this
/// can only be a call or a `use`.
const THE_PASS: &str = "group_properties_of";

/// `crates/outl-actions/src/outline.rs` as of commit `308d2321`, byte
/// for byte:
///
/// ```sh
/// git show 308d2321:crates/outl-actions/src/outline.rs \
///   > crates/outl-actions/tests/fixtures/outline-308d2321.rs.txt
/// ```
const FIXTURE: &str = "crates/outl-actions/tests/fixtures/outline-308d2321.rs.txt";

/// Files that project a node from the tree for a reader. Each one walks
/// a subtree, so each one has to take the pass rather than the accessor.
///
/// Scoped to the projections rather than swept workspace-wide: reading
/// one node's properties *once* is what `properties_of` is for, and
/// `page_properties`, the clipboard and the template instantiator each
/// do exactly that. Sweeping them in would turn this into an allowlist,
/// which is a second copy of the rule.
const PROJECTIONS: &[&str] = &[
    "crates/outl-actions/src/outline.rs",
    "crates/outl-actions/src/journal/render.rs",
];

/// The walk that **receives** a grouped map rather than building one.
///
/// `build_backlink_index` projects every citing block in the workspace,
/// so a per-node read here is `O(refs × properties)` — this was the
/// third instance and the ratchet is what found it. It takes the pass as
/// a parameter (`outline::workspace_properties` groups it), so it names
/// no `group_properties_of` of its own and only the first assertion
/// applies to it. Without this entry, moving the read out of
/// `project_outline_node_shallow` and into `walk_page` would be
/// invisible.
const THREADS_THE_PASS: &[&str] = &["crates/outl-actions/src/backlinks_index.rs"];

/// The one module allowed to read the map for a set of nodes.
const OWNER: &str = "crates/outl-actions/src/tree/props.rs";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// Every identifier in `src`, in source order, descending into every
/// delimited group.
///
/// The whole instrument. Idents are what a call is made of, so the
/// formatter cannot hide one and prose cannot forge one: a `//` comment
/// never reaches the token stream and a `///` one arrives as the string
/// literal of a `#[doc = "…"]` attribute.
fn idents(relative: &str, src: &str) -> Vec<String> {
    fn walk(stream: proc_macro2::TokenStream, out: &mut Vec<String>) {
        for tree in stream {
            match tree {
                proc_macro2::TokenTree::Ident(ident) => out.push(ident.to_string()),
                proc_macro2::TokenTree::Group(group) => walk(group.stream(), out),
                _ => {}
            }
        }
    }
    let stream: proc_macro2::TokenStream = src.parse().unwrap_or_else(|e| {
        panic!("{relative} does not lex as Rust ({e}) — it is being scanned as nothing at all")
    });
    let mut out = Vec::new();
    walk(stream, &mut out);
    out
}

/// What the ratchet reads off one file: how often it reaches the
/// per-node accessor, and whether it takes the shared pass at all.
struct Reads {
    per_node: usize,
    takes_the_pass: bool,
}

fn reads(relative: &str, src: &str) -> Reads {
    let names = idents(relative, src);
    Reads {
        per_node: names.iter().filter(|n| *n == PER_NODE_READ).count(),
        takes_the_pass: names.iter().any(|n| n == THE_PASS),
    }
}

fn source(root: &Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative)).unwrap_or_else(|e| {
        panic!("{relative} could not be read ({e}) — it is being scanned as nothing at all")
    })
}

#[test]
fn no_projection_walks_a_subtree_asking_the_property_map_per_node() {
    let root = workspace_root();

    let owner_src = source(&root, OWNER);
    assert!(
        idents(OWNER, &owner_src)
            .windows(2)
            .any(|pair| pair[0] == "fn" && pair[1] == THE_PASS),
        "{OWNER} no longer declares the subtree property pass. Point this \
         test at its new home rather than deleting it — the ratchet is the \
         reason the second scan stayed gone."
    );

    for (relative, builds_the_map) in PROJECTIONS
        .iter()
        .map(|path| (path, true))
        .chain(THREADS_THE_PASS.iter().map(|path| (path, false)))
    {
        let src = source(&root, relative);
        let found = reads(relative, &src);
        assert_eq!(
            found.per_node, 0,
            "{relative} reads the property map per node ({} call(s) to \
             `{PER_NODE_READ}`).\n\
             `Tree::properties_of` filters the whole workspace-wide map on \
             every call, so a walk that uses it is \
             `O(blocks × properties)` — 2,588 ms for one page on the \
             reference workspace. Collect the walk's ids and call \
             `crate::tree::group_properties_of` once instead.",
            found.per_node
        );
        if builds_the_map {
            assert!(
                found.takes_the_pass,
                "{relative} projects a subtree without taking the shared \
                 property pass — it is reading properties some third way, \
                 or it stopped reading them at all"
            );
        }
    }
}

/// The ratchet, run against the code it exists to refuse.
///
/// Both of the assertions above fail on [`FIXTURE`], and that had to be
/// shown rather than assumed: the first version of this ratchet
/// **passed** on it. Its pattern was a single-line string against a
/// chain rustfmt had already broken across three lines, and the one
/// assertion a full revert did fail was the one a doc comment satisfies.
/// A ratchet nobody has pointed at the defect is a ratchet nobody has
/// tested.
///
/// The counts are exact on purpose. The base file names the accessor a
/// third time, in a doc comment on `OutlineNode::properties`, and two is
/// what proves that mention is invisible here — the false positive that
/// a whitespace-tolerant string match would have had to carry.
#[test]
fn the_ratchet_fires_on_the_projection_it_replaced() {
    let src = source(&workspace_root(), FIXTURE);
    let found = reads(FIXTURE, &src);

    assert_eq!(
        found.per_node, 2,
        "the base file reads the property map per node in `project_node` \
         and in `project_outline_node_shallow`, and names the accessor \
         once more in prose. Two means the calls are seen and the prose is \
         not; three means a doc comment is being counted as a call, which \
         is how the old assertion on the pass passed vacuously."
    );
    assert!(
        !found.takes_the_pass,
        "the base file predates `{THE_PASS}`, so the second assertion has \
         to fail on it too. If this fixture mentions the pass, it is no \
         longer the code being replaced."
    );
}
