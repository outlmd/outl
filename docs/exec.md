# Running code blocks

A fenced code block in a bullet can run.
The output lands as a child bullet under the block, and running it again replaces that child instead of adding another.

```markdown
- ```lua
  local s = 0
  for i = 1, 4 do s = s + i * i end
  print(s)
  ```
  - > **result:** `30`
```

You write the first four lines.
outl writes the last one.
The result child is ordinary outline text, so it syncs, shows up in search and survives a round trip through any markdown editor.

## Run one

| Client | How |
|---|---|
| **TUI** | Select the block in Normal mode and press `g x`. On a block that is not a code block, `g x` opens the link under the cursor instead |
| **Desktop** | Click **▶ Run** on the block, or press `Cmd+Shift+X` in view mode. Inside the text editor `Cmd+Shift+X` is strikethrough, so commit the edit first |
| **Mobile** | Long-press the block and pick **Run `<language>`** |

The chord is `RunCodeBlock` in the shortcut catalog, so [Shortcuts](shortcuts.md) and [Client parity](client-parity.md) stay the owners of the key.
The TUI also has the `/run` command.

Write the block on one line with the opener to get started: type `` - ```lua `` and press `Enter`, then type the body.
A bullet whose first line is the fence is the shape every client expects.
See [Fenced code blocks](markdown-format.md#fenced-code-blocks-inside-a-bullet) for what the parser preserves verbatim inside it.

## Languages

The fence language picks the runtime.
Aliases fold to one name, so `` ```javascript ``, `` ```node `` and `` ```js `` are the same block.

| Write | Runs on | Output | Where it runs |
|---|---|---|---|
| `js`, `javascript`, `node`, `nodejs` | [Boa](https://boajs.dev), ES2015+ | `console.log` / `info` / `warn` / `error` lines. If the block prints nothing, the value of its last expression | TUI, desktop, mobile |
| `python`, `py`, `python3` | [RustPython](https://rustpython.github.io), a Python 3 subset | `print(...)` only. No numpy, pandas or native extensions | TUI, desktop, mobile |
| `lua` | Lua 5.4 | `print(...)`, or the value of a top-level `return` | TUI, desktop, mobile |
| `rust`, `rs` | `rustc` compiled to WebAssembly, run in a wasmtime sandbox | `println!`. Wrapped in `fn main() { … }` when you did not write one | TUI, desktop. **Not mobile** |
| `query` (alias `tasks`) | The workspace itself | Live embeds of matching blocks. See [Query code blocks](query.md) | Every client |
| `lisp`, `cl`, `common-lisp`, `elisp` | Steel (Scheme) | | **Only in a build that enables `lang-lisp`.** Off in every release build, because Steel cannot be sandboxed from outside: a `lisp` fence would be as trusted as a shell |

A fence in any other language (`bash`, `sql`, `typescript`, `yaml`, …) is syntax-highlighted on desktop and mobile and never runs.
Running one says `no runtime registered for language`.
A debug build of outl also registers `echo`, which prints the block back and exists to test the pipeline; release builds leave it out.
The mobile menu offers **Run** only for `js`, `python` and `lua`, so a long-press on a `rust` fence shows nothing to tap.

### Rust needs a toolchain

`rust` shells out to `rustc` on your `PATH` and needs the WebAssembly target once:

```sh
rustup target add wasm32-wasip1
```

Without it the result child says `sandbox: …` with the exact command to run.
The first run of a snippet compiles it (50 to 500 ms typical); the compiled module is cached by the hash of its source, so running it again is instant.

## What the result looks like

| Outcome | Result child |
|---|---|
| One line of output | ``> **result:** `30` `` |
| Several lines | `> **result:**` followed by a code fence holding the output |
| Nothing printed | ``> **result:** `(no output)` `` |
| Script failed (non-zero exit) | `> **result (exit 1):**` and the output |
| Script crashed (syntax error, uncaught exception) | `> **result (trap: …):**` and the message |
| outl could not run it (timeout, missing toolchain, unknown language) | ``> **result:** `error: …` `` |

When a script prints nothing on stdout and something on stderr, the stderr text is shown instead, so an error message is never swallowed.

`query` blocks answer differently: the child is `> **result:** (N blocks)` with one embed per hit.
`outl template run` (below) writes each output line as a child bullet under `> **result:**` instead of an inline code span.

## Auto-run

A block can re-run without you asking.

- **`query` fences always do.**
  Every page load runs them, because their output depends on the workspace and not on the fence body.
  `g x` on one says `query blocks auto-run, no manual execution needed`.
- **Any other block in the TUI** runs on page load and after each save when its bullet carries an `auto-run::` property.
  The property can hold any value; only its presence counts.
  The run is skipped when the fence body is unchanged since the last result, so navigating around is cheap.

  ```markdown
  - ```python
    print(sum(range(10)))
    ```
    auto-run:: true
    - > **result:** `45`
      source-hash:: sha256:…
  ```

  `source-hash::` is outl's own bookkeeping on the result child.
  Leave it alone; deleting it only forces one extra run.
- **Desktop and mobile ignore `auto-run::`.**
  They auto-run `query` fences and nothing else, so a `python` block that refreshes itself in the TUI shows its last result and waits for **Run** on the other two.
  Treat `auto-run::` as a TUI feature until that changes.

An auto-run failure never blocks the page.
The TUI status line says `auto-run skipped block N: …` and the rest of the page loads.

## Limits

| Limit | Value | What you see |
|---|---|---|
| Wall-clock time | 5 seconds (2 seconds on iOS) | `error: execution timed out after 5s` |
| Memory | 64 MiB, enforced for `lua` only | the script fails with a memory error |
| Files, network, processes | Not reachable from `js`, `python`, `lua` or `rust` | the call fails inside the script |

Execution is synchronous: the app waits for the block.
`js` and `python` cannot be interrupted from outside, so a block that never ends is abandoned at the deadline and its thread keeps running in the background.
After four abandoned workers outl refuses to start another until one finishes.
If `Run` starts answering with a sandbox error after a few infinite loops, restart the app.

A code block is not always yours.
A page that reached you over sync or from `outl import` can hold a fence somebody else wrote, which is the reason the sandbox is an allowlist and `lisp` is off.
A page's `auto-run::` block runs the moment you open it in the TUI, so read a block you did not write before opening that page on a machine where it matters.

## Callable templates

A `call:<name>` fence runs a [template](templates.md) with parameters instead of its own body:

````markdown
- ```call:calc-salary
  requested: 15000
  offered: 18000
  ```
````

| Client | Runs when |
|---|---|
| **TUI** | You press `g x` |
| **Desktop** | You click **▶ Run**, and again each time you finish editing the block |
| **Mobile** | You finish editing the block. The long-press menu has no Run entry for `call:` fences |

From a script or an agent: `outl template run <name> --page <slug> --block <id> --params key=value`, and `outl_template_run` over MCP.
Details and troubleshooting are in [Templates](templates.md#type-2-callable-templates).

## When it does not work

| You see | Why | Fix |
|---|---|---|
| `no runtime registered for language <lang>` | A language outl does not run, or one this build left out | Use a language from the table above. For `lisp`, build with the `lang-lisp` feature |
| `no code block or link under cursor` (TUI) | The selected bullet is not a fence and carries no link | Select the bullet that holds the opening `` ``` `` |
| `block is not a fenced code block` | The bullet's text does not start with a fence, usually a stray line above it | Put the fence on the bullet's first line |
| `code block has no language tag` | The opener is a bare `` ``` `` | Add the language: `` ```python `` |
| `sandbox: rustc not found on PATH` | No Rust toolchain on this machine | Install `rustup`, then `rustup target add wasm32-wasip1` |
| `execution timed out after 5s` | The script ran past the limit | Make it shorter, or move the work out of the note |
| Result never updates on desktop or mobile | The block has `auto-run::`, which only the TUI honors | Press **Run**, or switch the block to a `query` fence if it is one |
| Python import fails | RustPython ships a subset of the standard library, no packages | Do the work in `js` or `lua`, or run it outside outl |

## For contributors

Runtime trait, sandbox rules, the per-runtime timeout mechanism and how to add a language: [`crates/outl-exec/CLAUDE.md`](../crates/outl-exec/CLAUDE.md).
The shared entry point every client calls is `outl_actions::run_code_block`; the alias table is `outl_md::lang::KNOWN_ALIASES`, mirrored in `@outl/shared/highlight`.
