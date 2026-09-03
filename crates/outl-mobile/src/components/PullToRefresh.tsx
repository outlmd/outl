import { JSX, createSignal, onCleanup, onMount } from "solid-js";

interface PullToRefreshProps {
  children: JSX.Element;
  /** Pixels of pull needed to commit the refresh. */
  threshold?: number;
  /** Called when user releases past `threshold`. Should return a
   * promise; we display the spinner until it resolves. */
  onRefresh: () => Promise<void> | void;
  /** Element whose scroll position decides whether we may capture
   * pull. When omitted we use `window`. */
  scrollRoot?: () => HTMLElement | undefined;
}

/**
 * Native-feeling pull-to-refresh. Tracks pointer drag down from the
 * top of the scroll container; only captures when the container is
 * already scrolled to the top so vertical scroll keeps working
 * normally. Releases past `threshold` fire `onRefresh`.
 */
export function PullToRefresh(props: PullToRefreshProps): JSX.Element {
  const threshold = () => props.threshold ?? 72;
  const [pull, setPull] = createSignal(0);
  const [refreshing, setRefreshing] = createSignal(false);
  let container!: HTMLDivElement;
  let active = false;
  let captured = false;
  let startY = 0;
  let startX = 0;

  function scrollY(): number {
    const root = props.scrollRoot?.();
    if (root) return root.scrollTop;
    return window.scrollY;
  }

  function onPointerDown(e: PointerEvent) {
    if (refreshing()) return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    if (scrollY() > 2) return;
    if (
      (e.target as HTMLElement).closest(
        "textarea,input,button,[role='button']",
      )
    ) {
      return;
    }
    startY = e.clientY;
    startX = e.clientX;
    active = true;
    captured = false;
  }

  function onPointerMove(e: PointerEvent) {
    if (!active || refreshing()) return;
    const dy = e.clientY - startY;
    const dx = e.clientX - startX;
    if (!captured) {
      if (dy > 8 && scrollY() <= 2) {
        captured = true;
        container.setPointerCapture?.(e.pointerId);
      } else if (dy < -8 || Math.abs(dx) > 10) {
        // moved up or moved horizontally too much — bail. `dx` is
        // measured against the pointerdown X, not against 0, so an
        // ordinary downward drag starting anywhere on screen is not
        // mistaken for a horizontal swipe.
        active = false;
        return;
      } else {
        return;
      }
    }
    // Rubber band: scale offset to ease toward limit
    const scaled = dy <= threshold() ? dy : threshold() + (dy - threshold()) * 0.35;
    setPull(Math.max(0, scaled));
  }

  async function commit() {
    if (pull() >= threshold()) {
      setRefreshing(true);
      try {
        await props.onRefresh();
      } finally {
        setRefreshing(false);
        setPull(0);
      }
    } else {
      setPull(0);
    }
  }

  function onPointerUp() {
    if (!active) return;
    active = false;
    if (captured) commit();
  }

  function onPointerCancel() {
    if (!active) return;
    active = false;
    setPull(0);
  }

  onMount(() => {
    void container; // ref captured below
  });
  onCleanup(() => {
    active = false;
  });

  const progress = () => Math.min(1, pull() / threshold());

  return (
    <div
      ref={container}
      class="relative w-full"
      style={{ "touch-action": "pan-y" }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
    >
      {/* Pull indicator stays in normal layout flow at the top of
          the container so it doesn't fight with the iOS status bar. */}
      <div
        class="pointer-events-none absolute inset-x-0 top-0 z-10 flex items-center justify-center"
        style={{
          height: `${refreshing() ? threshold() : pull()}px`,
          opacity: refreshing() ? 1 : progress(),
          transition: refreshing() || pull() === 0 ? "all 220ms cubic-bezier(0.32, 0.72, 0, 1)" : "none",
        }}
      >
        <svg
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="var(--color-outl-accent)"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
          style={{
            transform: refreshing()
              ? "rotate(360deg)"
              : `rotate(${progress() * 360}deg)`,
            transition: refreshing()
              ? "transform 700ms linear"
              : "none",
            "animation": refreshing() ? "outl-spin 800ms linear infinite" : undefined,
          }}
        >
          <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
          <path d="M21 3v5h-5" />
        </svg>
      </div>
      <div
        style={{
          transform: `translateY(${refreshing() ? threshold() : pull()}px)`,
          transition: refreshing() || pull() === 0
            ? "transform 220ms cubic-bezier(0.32, 0.72, 0, 1)"
            : "none",
        }}
      >
        {props.children}
      </div>
    </div>
  );
}
