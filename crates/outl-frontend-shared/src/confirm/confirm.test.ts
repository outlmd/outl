import { describe, expect, it, vi } from "vitest";

import { createConfirmQueue } from "./index";

interface Req {
  message: string;
}

describe("createConfirmQueue", () => {
  it("holds the question until it is answered", async () => {
    const q = createConfirmQueue<Req>();
    expect(q.pending()).toBeNull();

    const asked = q.ask({ message: "merge?" });
    expect(q.pending()?.message).toBe("merge?");

    q.answer(true);
    await expect(asked).resolves.toBe(true);
    expect(q.pending()).toBeNull();
  });

  it("resolves false on a refusal rather than rejecting", async () => {
    const q = createConfirmQueue<Req>();
    const asked = q.ask({ message: "delete?" });
    q.answer(false);
    // Declining is an outcome, not a failure: a caller that `await`s
    // this must not have to wrap it in a try.
    await expect(asked).resolves.toBe(false);
  });

  it("refuses the open question when a second one arrives", async () => {
    const q = createConfirmQueue<Req>();
    const first = q.ask({ message: "first" });
    const second = q.ask({ message: "second" });

    // Two cards cannot be on screen at once, and a promise nobody can
    // reach is a flow that never finishes — so the one being replaced
    // ends on the side that writes nothing.
    await expect(first).resolves.toBe(false);
    expect(q.pending()?.message).toBe("second");

    q.answer(true);
    await expect(second).resolves.toBe(true);
  });

  it("refuses immediately when nothing can answer, and says so", async () => {
    const onUnanswerable = vi.fn();
    const q = createConfirmQueue<Req>({ canAnswer: () => false, onUnanswerable });

    const asked = q.ask({ message: "merge?" });

    // The alternative is a promise that never settles: the button
    // appears dead, no error is raised, and the `await` holds whatever
    // is behind it forever.
    await expect(asked).resolves.toBe(false);
    expect(q.pending()).toBeNull();
    expect(onUnanswerable).toHaveBeenCalledWith({ message: "merge?" });
  });

  it("queues normally while something can answer", async () => {
    const q = createConfirmQueue<Req>({ canAnswer: () => true });
    const asked = q.ask({ message: "merge?" });
    expect(q.pending()?.message).toBe("merge?");
    q.answer(true);
    await expect(asked).resolves.toBe(true);
  });

  it("answering nothing is a no-op", () => {
    const q = createConfirmQueue<Req>();
    expect(() => q.answer(true)).not.toThrow();
    expect(q.pending()).toBeNull();
  });

  it("carries a request whose own fields the queue knows nothing about", async () => {
    // The payload is a type parameter precisely so each client's chrome
    // fields stay its own; the queue must pass them through untouched.
    interface Rich {
      title: string;
      destructive: boolean;
    }
    const q = createConfirmQueue<Rich>();
    const asked = q.ask({ title: "Delete page", destructive: true });
    expect(q.pending()?.title).toBe("Delete page");
    expect(q.pending()?.destructive).toBe(true);
    q.answer(false);
    await expect(asked).resolves.toBe(false);
  });
});
