// Puts a raw capture on a soft background: a window card for desktop
// and the TUI, a rounded phone body for mobile. Rendered by Chromium,
// so there is no image toolchain to install.
import { readFileSync } from "node:fs";

const BG = {
  light: "linear-gradient(135deg, #ede9fe 0%, #e0e7ff 50%, #fce7f3 100%)",
  dark: "linear-gradient(135deg, #1e1b4b 0%, #111827 55%, #3b0764 100%)",
};

export async function frame(browser, { raw, out, kind, scheme, width, height, dpr }) {
  const src = `data:image/png;base64,${readFileSync(raw).toString("base64")}`;
  const phone = kind === "mobile";
  const pad = phone ? 72 : 64;
  const inner = phone
    ? `<div class="phone"><img src="${src}"></div>`
    : `<div class="win"><div class="bar"><i></i><i></i><i></i></div><img src="${src}"></div>`;
  const page = await browser.newPage({ deviceScaleFactor: dpr, viewport: { width: 800, height: 600 } });
  await page.setContent(`<!doctype html><style>
    html,body{margin:0}
    #stage{display:inline-block;padding:${pad}px;background:${BG[scheme]}}
    img{display:block;width:${width}px;height:${height}px}
    .win{border-radius:14px;overflow:hidden;box-shadow:0 30px 80px rgba(0,0,0,.35),0 0 0 1px rgba(127,127,127,.25)}
    .bar{height:36px;display:flex;gap:8px;align-items:center;padding:0 16px;background:${scheme === "dark" ? "#27272a" : "#f4f4f5"}}
    .bar i{width:12px;height:12px;border-radius:50%;background:#ff5f57}.bar i+i{background:#febc2e}.bar i+i+i{background:#28c840}
    .phone{padding:14px;border-radius:64px;background:${scheme === "dark" ? "#0a0a0a" : "#1f1f1f"};box-shadow:0 30px 80px rgba(0,0,0,.35)}
    .phone img{border-radius:50px}
  </style><div id="stage">${inner}</div>`);
  await page.locator("img").evaluate((img) => img.decode());
  await page.locator("#stage").screenshot({ path: out });
  await page.close();
}
