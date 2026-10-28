//! Template engine — structural templates + callable code blocks.
//!
//! A **template** is any page with a non-empty `template::` property.
//! The property value is the invocation name (what the user types
//! after `/template`). The page's outline is the template body.
//!
//! Two invocation modes:
//!
//! - **Structural** (`/template <name>`): deep-copy the template's
//!   subtree under the target block with built-in variable
//!   substitution. See [`instantiate::instantiate_template`].
//! - **Callable** (` ```call:<name> `): resolve the template's code
//!   block for execution with params. See [`call::resolve_call`].
//!
//! Traceability: structural instances get `from-template:: <slug>` on
//! each root block, and callable sites carry a ` ```call:<name> `
//! fence. Neither is a plain `[[ref]]` in the block text, so
//! [`crate::backlinks::backlinks_for_page`] recognizes both explicitly
//! when the target page is a template — that's how the template page's
//! backlinks panel surfaces every place it was rendered or instantiated.

/// Property key marking a page as a template.
pub const TEMPLATE_KEY: &str = "template";

/// Property key on instantiated blocks recording which template
/// they were created from.
pub const FROM_TEMPLATE_KEY: &str = "from-template";

/// Property key declaring a callable template's parameter names
/// (comma-separated).
pub const PARAMS_KEY: &str = "params";

/// Reserved template name for the daily journal body. A page with
/// `template:: journal` is stamped into a fresh daily note
/// automatically the first time it is opened (see
/// [`crate::page::open_journal`]).
pub const JOURNAL_TEMPLATE_NAME: &str = "journal";

/// Parse a comma-separated `params::` property value into a list of
/// trimmed, non-empty parameter names. Same shape the `alias::` page
/// property uses; the splitter lives once in `outl_md`.
pub(crate) fn parse_param_list(value: &str) -> Vec<String> {
    outl_md::parse_alias_list(value)
}

pub mod call;
pub mod instantiate;
pub mod list;
pub mod run;
pub mod vars;

pub use call::{
    call_target_name, inject_call_params, parse_call_params, resolve_call, CallResolution,
};
pub use instantiate::instantiate_template;
pub use list::{list_templates, TemplateEntry};
pub use run::{parse_call_invocation, run_callable_block};
