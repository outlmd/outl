import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";

import {
  diagramFailureView,
  formatDiagramError,
  linkDisposition,
  overDiagramLimit,
} from "./policy";
import {
  createPeleRenderer,
  type DiagramFailure,
  type DiagramHooks,
  type DiagramLink,
  type DiagramMount,
} from "./renderer";

/**
 * Built-in mermaid fence.
 *
 * The host stays mounted so Pele can measure it. It is hidden when the
 * fence falls back to source. Pele paints its own `.pele-error` into
 * that host (https://pele.run/api); we clear it and show one line of
 * our own, assigned with `textContent` (https://pele.run/security).
 */
export function DiagramFence(props: {
  source: string;
  onOpenPage?: (name: string) => void;
  onLinkClick?: (href: string) => void;
  fallback: JSX.Element;
}): JSX.Element {
  const [view, setView] = createSignal<"pending" | "diagram" | "code" | "error">("pending");
  const [errorLine, setErrorLine] = createSignal("");
  let host: HTMLDivElement | undefined;
  let errorEl: HTMLParagraphElement | undefined;
  let mounted: DiagramMount | undefined;
  let links: DiagramLink[] = [];
  let generation = 0;
  const renderer = createPeleRenderer();

  function clearHost() {
    if (host) host.textContent = "";
  }

  function dropMount() {
    const active = mounted;
    mounted = undefined;
    links = [];
    active?.destroy();
    clearHost();
  }

  function hooksFor(token: number): DiagramHooks {
    return {
      onLinks: (next: DiagramLink[]) => {
        if (token !== generation) return;
        links = next;
      },
      onError: (failure: DiagramFailure) => {
        if (token !== generation) return;
        showFailure(failure);
      },
    };
  }

  function showFailure(failure: DiagramFailure) {
    dropMount();
    if (diagramFailureView(failure.code) === "code") {
      setErrorLine("");
      setView("code");
      return;
    }
    setErrorLine(formatDiagramError(failure.line, failure.column, failure.message));
    setView("error");
  }

  async function show(source: string) {
    const token = ++generation;
    if (overDiagramLimit(source)) {
      dropMount();
      setErrorLine("");
      setView("code");
      return;
    }
    let supported = false;
    try {
      supported = await renderer.supports(source);
    } catch {
      supported = false;
    }
    if (token !== generation) return;
    if (!supported) {
      dropMount();
      setErrorLine("");
      setView("code");
      return;
    }
    if (!host) return;
    setView("diagram");
    const hooks = hooksFor(token);
    try {
      if (mounted) {
        await mounted.update(source, hooks);
      } else {
        const next = await renderer.mount(host, source, hooks);
        if (token !== generation || view() !== "diagram") {
          next.destroy();
          return;
        }
        mounted = next;
      }
    } catch (error) {
      if (token !== generation) return;
      showFailure({
        message: error instanceof Error ? error.message : String(error),
        line: 0,
        column: 0,
        code: "unknown",
      });
    }
  }

  createEffect(() => {
    void show(props.source);
  });

  createEffect(() => {
    const text = view() === "error" ? errorLine() : "";
    if (errorEl) errorEl.textContent = text;
  });

  onCleanup(() => {
    generation += 1;
    dropMount();
  });

  function onClick(event: MouseEvent) {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const anchor = target.closest("a");
    if (!anchor || !host?.contains(anchor)) return;
    const raw = anchor.getAttribute("data-href") ?? anchor.getAttribute("href") ?? "";
    const hit = links.find((link) => link.href === raw);
    const link = hit ?? {
      id: "",
      href: raw,
      internal: anchor.classList.contains("internal-link"),
    };
    const disposition = linkDisposition(link);
    if (disposition === "page") {
      if (!props.onOpenPage) return;
      event.preventDefault();
      event.stopPropagation();
      props.onOpenPage(link.href);
      return;
    }
    if (disposition === "external") {
      if (!props.onLinkClick) return;
      event.preventDefault();
      event.stopPropagation();
      props.onLinkClick(link.href);
    }
  }

  return (
    <>
      <div
        class="outl-diagram"
        ref={host}
        onClick={onClick}
        style={{ display: view() === "diagram" ? "block" : "none" }}
      />
      <Show when={view() === "code" || view() === "error"}>{props.fallback}</Show>
      <Show when={view() === "error"}>
        <p
          class="outl-diagram-error"
          data-diagram-error
          ref={(el) => {
            errorEl = el;
            el.textContent = errorLine();
          }}
        />
      </Show>
    </>
  );
}
