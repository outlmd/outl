/**
 * Visual parity guards: the few class choices desktop and mobile must
 * make the same way. Read as text, because rendering either row needs
 * its whole client.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const crates = resolve(__dirname, "../../..");
const read = (p: string) => readFileSync(resolve(crates, p), "utf8");

/** Every `.ts` / `.tsx` under a client's `src/`, recursively. */
function clientSources(client: string): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) {
        walk(full);
      } else if (/\.tsx?$/.test(entry)) {
        out.push(full);
      }
    }
  };
  walk(resolve(crates, client, "src"));
  return out;
}

describe("desktop and mobile rows", () => {
  it("block text and its textarea share one line height token", () => {
    for (const f of [
      "outl-desktop/src/components/BlockRow.tsx",
      "outl-mobile/src/components/BlockBody.tsx",
      "outl-mobile/src/components/BlockTextarea.tsx",
    ]) {
      expect(read(f), f).toContain("leading-(--outl-block-leading)");
    }
  });

  it("task markers use the theme's task colours, not the accent", () => {
    const mobile = read("outl-mobile/src/components/BlockMarkers.tsx");
    const desktop = read("outl-desktop/src/components/BlockRow.tsx");
    for (const src of [mobile, desktop]) {
      expect(src).toContain("--color-outl-todo-open-fg");
      expect(src).toContain("--color-outl-todo-done-fg");
    }
  });
});

/**
 * RFC 0377 section 6: the crash-reporting words have one owner per
 * medium, and for the GUI that owner is `@outl/shared/onboarding`.
 *
 * The reason is not tidiness. Three of outl's privacy declarations go
 * to an app store, so a client that phrases this itself is how one of
 * them becomes a false statement to Apple — and nothing in a client's
 * own build could notice.
 */
describe("crash-reporting disclosure", () => {
  it("both GUI clients mount the shared strip at their shell", () => {
    // Shell level on purpose: desktop's `AppShell` and mobile's `App`
    // render once per session, after onboarding, so the strip is not a
    // per-page banner and its dismissed check is not per navigation.
    expect(read("outl-desktop/src/components/AppShell.tsx")).toContain(
      "<CrashReportsNotice />",
    );
    expect(read("outl-mobile/src/App.tsx")).toContain("<CrashReportsNotice />");
  });

  it("both GUI clients read the shared Settings copy", () => {
    expect(read("outl-desktop/src/components/SettingsModal.tsx")).toContain(
      "CRASH_REPORTS_SETTING",
    );
    expect(read("outl-mobile/src/components/SettingsSheet.tsx")).toContain(
      "CRASH_REPORTS_SETTING",
    );
  });

  it("no client source spells out what a crash report carries", () => {
    // Catches the copy being inlined, which is how this drifts in
    // practice: somebody tweaks a sentence in one client and the other
    // (and the store declaration) keep the old one. It does not catch
    // an arbitrary rewrite — the positive checks above are what pin
    // that the shared constants are the ones being rendered.
    const phrases = [
      "notes are never",
      "never part of it",
      "never included",
      "it sends the crash",
      "sends what broke",
    ];
    for (const client of ["outl-desktop", "outl-mobile"]) {
      for (const file of clientSources(client)) {
        const lower = readFileSync(file, "utf8").toLowerCase();
        for (const phrase of phrases) {
          expect(lower, `${file} spells out "${phrase}"`).not.toContain(phrase);
        }
      }
    }
  });
});
