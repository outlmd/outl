/**
 * One block move in flight at a time.
 *
 * A held chord repeats faster than a move round-trip, and a move reads
 * the outline the previous one has not repainted yet, so an ungated
 * repeat sees stale edges. The gate runs `run` synchronously, drops
 * every call that arrives while one is pending, and releases in
 * `finally` so a rejected command cannot wedge it shut.
 */
export function createMoveGate(): (run: () => Promise<void>) => Promise<void> {
  let busy = false;
  return async (run) => {
    if (busy) return;
    busy = true;
    try {
      await run();
    } finally {
      busy = false;
    }
  };
}
