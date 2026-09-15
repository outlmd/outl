# Templates

Templates let you stamp a reusable block structure onto any page.
A template is just a page with a `template::` property — no special
folder, no file-based config.

There are two kinds, and you pick by what you want back:

- **Structural** ([Type 1](#type-1-structural-templates)) — clone a
  block structure (a 1:1 agenda, an interview checklist) into the
  current page.
  You get editable blocks.
- **Callable** ([Type 2](#type-2-callable-templates)) — run a code
  block with parameters and see its output inline (a salary calc, a
  units converter).
  You get a computed result.

The daily journal is itself a template — see
[Journal template](#journal-template).

> **Why a template is a page with a property and not a new op:** [RFC 0146](rfcs/0146-template-engine.md).

## Surface scope

Templates are one feature shared across every client through
`outl-actions`, but the surfaces that expose each mode are still
filling in:

- **Structural** instantiation is reachable from the TUI slash menu,
  the CLI (`outl template apply`), MCP (`outl_template_apply`), and any
  plugin (`ctx.template.instantiate`).
- **Callable** execution runs wherever a code block can run — the TUI
  (`gx`), and the desktop / mobile **Run** button.

The engine itself (resolution, substitution, `call:` execution) is
identical everywhere; only the affordance that triggers it varies by
client.
If your client doesn't surface a picker yet, a plugin reaches the same
code path — see [below](#using-templates-from-plugins).

## Defining a template

Add `template:: <name>` to any page.
The page's outline is the template body.

```markdown
templates/interview
template:: interview

- **Candidate:**
- **Role:**
- **Base salary:**
  - TODO #fup [[@candidate]] send feedback [[tomorrow]]
```

The value of `template::` is the invocation name — what you type
after `/template` in the TUI or pass to `outl template apply`.

## Type 1: Structural templates

Clone a template's subtree into the current page.
These surfaces trigger it today: the TUI slash menu, the CLI, MCP, and
plugins (`ctx.template.instantiate`, see
[below](#using-templates-from-plugins)).

### Worked example

Say you keep a standup template:

```markdown
templates/standup
template:: standup

- **Standup {{date}}**
  - Yesterday:
  - Today:
  - Blockers:
```

On the journal `2026-07-08`, select a host block and run
`/template standup`.
The template's subtree is deep-copied under that block, with `{{date}}`
resolved to the **page's** date and a `from-template::` marker stamped
on each root clone for traceability:

```markdown
- host block
  - **Standup 2026-07-08**
    from-template:: standup
    - Yesterday:
    - Today:
    - Blockers:
```

The cloned blocks are fully editable — there's no live link back to the
template after instantiation.
The `from-template::` property is what surfaces this instance in the
template page's backlinks panel (see [Traceability](#traceability)); it
never appears inline in the rendered `.md` body.

### TUI

Type `/template <name>` (or `/tpl <name>`) in the slash menu.
The template blocks land as children of the selected block.

### CLI

```sh
outl template apply interview --page 2026-07-08
```

Optional `--block <id>` targets a specific block as parent.
When omitted, blocks are appended to the end of the page.

### MCP

```json
{
  "name": "outl_template_apply",
  "arguments": {
    "name": "interview",
    "page": "2026-07-08"
  }
}
```

## Built-in variables

Template text supports substitution tokens, replaced at
instantiation time:

| Token | Replaced with |
|---|---|
| `{{date}}` | Date of the target page (journal) or today |
| `{{today}}` | Today's date (ISO `YYYY-MM-DD`) |
| `{{yesterday}}` | Yesterday's date |
| `{{tomorrow}}` | Tomorrow's date |
| `{{page}}` | Slug of the target page |
| `{{time}}` | Current wall-clock `HH:MM` |

Unknown tokens are left verbatim so typos are visible.

Example template:

```markdown
- Meeting scheduled for [[{{date}}]]
  - Created at {{time}}
  - TODO review by [[{{tomorrow}}]]
```

## Type 2: Callable templates

A callable template has a fenced code block as its "renderer" and
declares parameters via `params::`.
This replaces Roam's `{{roam/render}}` without needing a
ClojureScript runtime.

### Definition

```markdown
templates/calc-salary
template:: calc-salary
params:: requested, offered

- ```python
  requested = int(params["requested"])
  offered = int(params["offered"])
  print(f"CLT: {requested} | PJ: {offered}")
  ```
```

### Inline call

In any block, use a `call:<name>` fence:

<pre><code>```call:calc-salary
requested: 15000
offered: 18000
```</code></pre>

The parser resolves the template, extracts its code block, injects
`params` from the YAML body, and executes via `outl-exec` (same
runtimes: Python, JS, Lua, Rust).
The result lands as a `> **result:**` subtree under the call block,
and re-running replaces it rather than accumulating.

**Running it — the same `call:` block works in every editor:**

- **TUI** — press `gx` on the block.
- **Desktop / mobile** — the **Run** button on the code block.
- **Everywhere** — finishing an edit on the block re-runs it
  automatically, so the result always reflects the params you just
  typed (no manual re-run needed).

Params are injected as JSON, so a value containing a quote or
backslash can't break — or inject into — the generated program.
The language is canonicalized, so `py` / `python3` / `node` all
receive the `params` prelude.
The `/template <name> key=value` slash command (TUI) runs the same
path when you don't want a persistent `call:` block.

### Resolve (CLI / MCP)

Check what a callable template contains without executing it:

```sh
outl template resolve calc-salary
```

Returns the language, source code, and declared params.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `template <name> not found` | No page has `template:: <name>` (typo, or the page's `template::` value differs from what you typed) | Check the exact value of the `template::` property on the template page — it's the invocation name, not the slug |
| `template <name> has no code block` | You invoked a callable path (`call:<name>` fence, or `/template <name> key=value`) on a template that has no fenced code block | Add a code fence to the template, or invoke it structurally (`/template <name>` with no params) to deep-copy the subtree instead |
| `no runtime for <lang>` | The template's code block uses a language this build doesn't link a runtime for | Use a supported language (Python, JS, Lua, Rust), or build with the matching `outl-exec` runtime feature enabled (`lang-lisp` is opt-in) |
| Two pages resolve to the wrong body | Two template pages share the same `template:: <name>` | Resolution picks the first in tree order and logs a warning; `list_templates` flags the collision (`duplicate`). Rename one of the templates |

Callable vs structural is decided by whether the template **has a
runnable code block**, not by whether it declares `params::`.
A code-block template with no `params::` still executes when called —
it just receives an empty `params`.

## Traceability

The template page's backlinks panel (`B` in the TUI) lists every
place the template was used — no hand-written `[[link]]` needed.
Two channels feed it:

- **Structural instances** carry `from-template:: <slug>` on each
  root block.
- **Callable sites** carry a `call:<name>` fence.

Neither is a plain `[[ref]]`, so the backlinks matcher recognizes
both explicitly when the target page is a template.

## Journal template

The daily journal is itself a template: a page `templates/journal`
with `template:: journal`.
`outl init` creates it (not a `templates/journal.md` file), and
opening a fresh daily note stamps its outline automatically — the
built-in variables resolve against that day's date.

The auto-stamp is **untraced**: every daily comes from the same
template, so a `from-template::` marker would be noise on every
note (and would flood the journal template's backlinks).

Existing workspaces with a customized `templates/journal.md`
migrate its body into the page on `init` (best-effort).

## API surface

All template logic lives in `outl-actions` so every client shares
the same implementation:

| Function | Purpose |
|---|---|
| `outl_actions::list_templates` | List all pages with `template::` (each entry flags `duplicate` when a name is shared) |
| `outl_actions::instantiate_template` | Deep-copy a template's subtree under a target block |
| `outl_actions::resolve_call` | Resolve a callable template's code block + params |
| `outl_actions::parse_call_params` | Parse the `key: value` body of a `call:` block |
| `outl_actions::inject_call_params` | Build the runnable source with a safe JSON `params` binding |
| `outl_actions::call_target_name` | The template name invoked by a `call:<name>` fence |

Variable substitution (`substitute_vars`) is `pub(crate)` — internal to
the engine, not part of the client-facing surface.
The template lookup (`outl_actions::template::list::find_template_by_name`)
is public; it resolves the first page in tree order and logs a
`tracing::warn!` when a `template:: <name>` is shared by more than one
page.

Constants:

| Constant | Property key |
|---|---|
| `TEMPLATE_KEY` | `template` |
| `FROM_TEMPLATE_KEY` | `from-template` |
| `PARAMS_KEY` | `params` |

## Using templates from plugins

Plugins can list and instantiate templates via `ctx.template`:

```ts
const templates = await ctx.template.list();
// → [{ name: "interview", slug: "templates-interview", params: [] }]

await ctx.template.instantiate("interview", targetBlockId);
// Deep-copies the template's subtree under the target block
```

`ctx.template.list()` needs `read-page` permission;
`ctx.template.instantiate()` needs `write-page`.

See the [Template Stamper](plugin-examples/template-stamper.md) example plugin
and the [Plugin API](plugin-api.md) reference for the full `ctx.template` surface.
