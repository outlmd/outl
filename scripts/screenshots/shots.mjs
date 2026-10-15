// The shot list. Each shot runs once per color scheme. `link` opens a
// page the way a cold-start `outl://` deep link does; `act` drives the
// UI before the capture.
//
// Left out on purpose, because the result would show a bug rather than
// the feature:
// - mobile block refs and embeds: outl-mobile never calls
//   `resolve_embeds`, so `((blk-…))` renders as the raw handle.
// - desktop pages that carry page properties are reached by clicking a
//   ref, never through the sidebar, quick switcher or a deep link:
//   AppShell.applyView drops `page_properties` (and skips embeds).
// - block refs / embeds and query results in general: their handles and
//   results only exist after the workspace is built, so the fixture
//   can't name them.

const day = (fx, by) => {
  const [y, m, d] = fx.today.split("-").map(Number);
  const t = new Date(y, m - 1, d + by);
  const p = (n) => String(n).padStart(2, "0");
  return `${t.getFullYear()}-${p(t.getMonth() + 1)}-${p(t.getDate())}`;
};
const daily = (by) => (fx) => ({ kind: "daily", date: day(fx, by) });

/** Click a `[[ref]]` in the open page by its visible label. */
const clickRef = (label) => async (page) => {
  await page.getByText(label, { exact: true }).first().click();
  await page.waitForTimeout(900);
};
const parkCursor = async (page) => {
  await page.keyboard.press("Escape");
  await page.mouse.click(5, 880);
};
const toggleSidebar = (page) => page.getByTitle(/^Toggle sidebar/).click();
const seq = (...steps) => async (page, fx) => {
  for (const s of steps) await s(page, fx);
};

export const desktopShots = [
  { name: "journal", act: seq(toggleSidebar, parkCursor) },
  { name: "project", act: seq(clickRef("Project Atlas"), parkCursor) },
  { name: "person-backlinks", act: seq(clickRef("@Ana Costa"), parkCursor) },
  { name: "checklist", link: daily(3), act: seq(clickRef("Atlas/Launch checklist"), parkCursor) },
  { name: "reading-list", link: daily(-1), act: seq(clickRef("Reading list"), parkCursor) },
  { name: "architecture", act: seq(clickRef("Project Atlas"), clickRef("Atlas/Architecture"), parkCursor) },
  {
    name: "quick-switcher",
    act: seq(parkCursor, async (page) => {
      await page.keyboard.press("Control+p");
      await page.waitForTimeout(400);
      await page.keyboard.type("atlas", { delay: 40 });
    }),
  },
  {
    name: "command-palette",
    act: seq(parkCursor, async (page) => {
      await page.keyboard.press("Control+k");
    }),
  },
];

const tab = (label) => async (page) => {
  await page.getByRole("button", { name: label, exact: true }).click();
  await page.waitForTimeout(700);
};

export const mobileShots = [
  { name: "journal" },
  { name: "project", link: () => ({ kind: "page", slug: "project-atlas" }) },
  { name: "person-backlinks", link: () => ({ kind: "page", slug: "ana-costa" }) },
  { name: "checklist", link: () => ({ kind: "page", slug: "atlas-launch-checklist" }) },
  { name: "tasks", act: tab("Tasks") },
  { name: "pages", act: tab("Pages") },
  {
    name: "search-blocks",
    act: seq(tab("Search"), async (page) => {
      await page.getByPlaceholder(/^Search blocks/).pressSequentially("sync", { delay: 40 });
      await page.waitForTimeout(600);
    }),
  },
  {
    name: "settings",
    act: async (page) => {
      await page.getByRole("button", { name: "Settings" }).first().click();
      await page.waitForTimeout(800);
      // The sheet can open scrolled to its end; show it from the top.
      await page.evaluate(() => {
        for (const el of document.querySelectorAll("*")) if (el.scrollTop > 0) el.scrollTop = 0;
      });
    },
  },
];

/** Framed shots also copied to hero/ for the release page. */
export const HERO = [
  "desktop-journal-light",
  "desktop-person-backlinks-dark",
  "desktop-project-light",
  "desktop-quick-switcher-dark",
  "mobile-iphone-journal-light",
  "mobile-iphone-search-blocks-dark",
  "mobile-android-person-backlinks-light",
  "tui-journal-light",
  "tui-person-backlinks-dark",
];
