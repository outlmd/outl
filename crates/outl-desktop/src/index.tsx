/* @refresh reload */
import { render } from "solid-js/web";
import { attachConsole } from "@tauri-apps/plugin-log";
import { bootMark } from "@outl/shared/boot";

import App from "./App";
import "./styles.css";

// The earliest point a module can report. Everything before this is the
// document and the inline palette script; everything after is the app.
// Reported here rather than in a component's `onMount` so the number
// brackets Solid's own mount cost instead of hiding it.
bootMark("html_parsed");

// Forward Rust logs (the `Webview` target of tauri-plugin-log) into the browser
// console, so backend `log::` lines show up in the webview devtools alongside
// the frontend's own `console.*`. Best-effort: outside a Tauri webview it no-ops.
void attachConsole().catch(() => {});

const mount = () => render(() => <App />, document.getElementById("root") as HTMLElement);

// `?mock=1` in `bun run dev` swaps the Tauri backend for an in-memory
// workspace so the UI runs in a plain browser. Dev builds only: the
// branch, and the mock with it, is dropped from production bundles.
if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) {
  void import("@outl/shared/mock").then((m) => {
    if (m.wantsMock()) m.installMockBackend();
    mount();
  });
} else {
  mount();
}
