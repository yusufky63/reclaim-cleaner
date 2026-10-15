// Captures README screenshots from the running dev app over the WebView2 DevTools protocol.
//
// 1. Start the app with a debugging port (PowerShell):
//      $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333"; pnpm tauri dev
// 2. Run: node scripts/screenshots.mjs [--out docs/screenshots] [--scan]
//
// It drives the real app (real scans of this PC), switches theme and width, and saves PNGs.
// No browser is launched, so there is nothing to clean up afterwards.

import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

const args = process.argv.slice(2);
const outDir = args.includes("--out") ? args[args.indexOf("--out") + 1] : "docs/screenshots";
const doScan = args.includes("--scan");
const port = process.env.CDP_PORT ?? "9333";
const pages = (args.includes("--pages") ? args[args.indexOf("--pages") + 1] : "overview,explorer,junk,projects,large,apps,tools,settings").split(",");
const themes = (args.includes("--themes") ? args[args.indexOf("--themes") + 1] : "light,dark").split(",");
const widths = (args.includes("--widths") ? args[args.indexOf("--widths") + 1] : "1440").split(",").map(Number);
const lang = args.includes("--lang") ? args[args.indexOf("--lang") + 1] : null;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function target() {
  for (let i = 0; i < 120; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = list.find((t) => t.type === "page" && /localhost:1420|tauri\.localhost/.test(t.url));
      if (page) return page;
    } catch {
      // The app is still starting.
    }
    await sleep(1000);
  }
  throw new Error(`no app page on port ${port}`);
}

function connect(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let id = 0;
  const pending = new Map();
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message));
      else resolve(msg.result);
    }
  });
  const ready = new Promise((r) => ws.addEventListener("open", r));
  const send = async (method, params = {}) => {
    await ready;
    const n = ++id;
    ws.send(JSON.stringify({ id: n, method, params }));
    return new Promise((resolve, reject) => pending.set(n, { resolve, reject }));
  };
  return { send, close: () => ws.close() };
}

const t = await target();
const cdp = connect(t.webSocketDebuggerUrl);
const evaluate = async (expression) => {
  const r = await cdp.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.text + " " + (r.exceptionDetails.exception?.description ?? ""));
  return r.result.value;
};

await mkdir(outDir, { recursive: true });
await evaluate(`(async () => { for (let i = 0; i < 50 && !window.__reclaim; i++) await new Promise(r => setTimeout(r, 200)); return !!window.__reclaim })()`);
if (lang) await evaluate(`window.__reclaimSetLang(${JSON.stringify(lang)})`);
if (doScan) {
  await evaluate(`(async () => { const s = window.__reclaim.getState(); await Promise.all([s.scanAll(), s.scanExplorer(s.info.userProfile)]); return true })()`);
}

for (const width of widths) {
  await cdp.send("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: false });
  for (const theme of themes) {
    await evaluate(`document.documentElement.setAttribute('data-theme', ${JSON.stringify(theme)})`);
    for (const page of pages) {
      await evaluate(`window.__reclaim.getState().go(${JSON.stringify(page)}); document.getElementById('main')?.scrollTo(0, 0)`);
      await sleep(900);
      const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
      const file = join(outDir, `${page}-${theme}${widths.length > 1 ? `-${width}` : ""}${lang ? `-${lang}` : ""}.png`);
      await writeFile(file, Buffer.from(shot.data, "base64"));
      console.log("saved", file);
    }
  }
}
await cdp.send("Emulation.clearDeviceMetricsOverride");
// Put the user's own theme and language back.
await evaluate(`(() => {
  const s = window.__reclaim.getState().settings;
  const root = document.documentElement;
  if (s.theme === "light" || s.theme === "dark") root.setAttribute("data-theme", s.theme); else root.removeAttribute("data-theme");
  window.__reclaimSetLang(s.language);
  window.__reclaim.getState().go("overview");
})()`);
cdp.close();
