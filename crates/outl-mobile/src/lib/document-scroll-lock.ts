/**
 * Keeps the app shell parked at the top of the WKWebView.
 *
 * **The problem.** This client is a shell, not a page: the only scroller
 * is the outline (`<main class="ios-scroll">`), and the journal chrome
 * (`JournalChrome`, the week strip) is ordinary content above it. The
 * WKWebView scrolls the *document* on its own anyway — when the keyboard
 * comes up it scrolls the visual viewport to keep the caret in view, and
 * that scroll is not CSS overflow at all: measured on an iPhone 17 Pro
 * simulator, `document.scrollingElement.scrollTop` reached 384 while
 * `scrollHeight === clientHeight === 874`. `overflow: hidden` on
 * `html` / `body` does not stop it, and neither does making `#root`
 * `position: fixed` — iOS moves fixed boxes with the visual viewport.
 *
 * While the keyboard is up that scroll is what keeps the line you are
 * typing visible, so it must be left alone. What must not survive is the
 * **leftover**: when the scroll is not put back, the header is parked
 * under the clock and the Dynamic Island, and every later drag moves the
 * whole page under the status bar. That is what the user sees as "the top
 * bar keeps riding up".
 *
 * **The rule, therefore:** with nothing focused for text entry, the
 * document sits at the top. Nothing else here decides what is scrollable.
 */

/** Is the caret in something that raises a keyboard? */
export function isEditing(doc: Document): boolean {
  const el = doc.activeElement as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "TEXTAREA" || tag === "INPUT" || el.isContentEditable === true;
}

/** Does the document sit anywhere other than its origin? */
export function isScrolled(doc: Document): boolean {
  const se = doc.scrollingElement;
  if (!se) return false;
  return se.scrollTop !== 0 || se.scrollLeft !== 0;
}

/**
 * Put the document back at the origin unless the keyboard owns it.
 *
 * Exported for the test: the behaviour worth pinning is the decision,
 * not the listener wiring.
 */
export function resetDocumentScroll(doc: Document): boolean {
  if (isEditing(doc)) return false;
  if (!isScrolled(doc)) return false;
  const se = doc.scrollingElement as HTMLElement;
  se.scrollTop = 0;
  se.scrollLeft = 0;
  return true;
}

/**
 * Wires the lock once, at boot. Three signals, because WKWebView is not
 * consistent about which it sends: the document's own `scroll`, the
 * visual viewport moving or resizing (keyboard animations), and losing
 * focus from a text field (the keyboard going away).
 */
export function installDocumentScrollLock(win: Window = window): () => void {
  const doc = win.document;
  const reset = () => resetDocumentScroll(doc);
  // After a blur the keyboard animates out, so the scroll settles a few
  // frames later; one immediate pass plus one settled pass covers both.
  const resetSettled = () => {
    reset();
    win.setTimeout(reset, 350);
  };

  win.addEventListener("scroll", reset, { passive: true });
  doc.addEventListener("focusout", resetSettled);
  const vv = win.visualViewport;
  vv?.addEventListener("resize", resetSettled);
  vv?.addEventListener("scroll", reset);

  return () => {
    win.removeEventListener("scroll", reset);
    doc.removeEventListener("focusout", resetSettled);
    vv?.removeEventListener("resize", resetSettled);
    vv?.removeEventListener("scroll", reset);
  };
}
