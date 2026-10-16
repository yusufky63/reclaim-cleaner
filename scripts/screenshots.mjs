// Captures README screenshots from the running dev app over the WebView2 DevTools protocol.
//
// Screenshots that go into the repository never show a real PC: build the demo profile with
// `node scripts/demo-profile.mjs`, start the app on it as that script prints, then run
//   node scripts/screenshots.mjs --demo [C:\ReClaimDemo] [--out docs/screenshots]
// `--demo` refuses to run unless the app uses the demo profile. It scans the profile, then drops
// what still comes from the machine itself: results outside the profile (system folders other
// than Windows', installed apps' data), installers from the real Downloads folder, removable
// drives. Without `--demo` the script captures whatever the app shows, for local checks only.
//
// It switches theme and width over the WebView2 DevTools protocol and saves PNGs. No browser is
// launched, so there is nothing to clean up afterwards.

import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

const args = process.argv.slice(2);
const outDir = args.includes("--out") ? args[args.indexOf("--out") + 1] : "docs/screenshots";
const demo = args.includes("--demo");
const demoRoot = demo && args[args.indexOf("--demo") + 1] && !args[args.indexOf("--demo") + 1].startsWith("--") ? args[args.indexOf("--demo") + 1] : "C:\\ReClaimDemo";
const port = process.env.CDP_PORT ?? "9333";
// Apps, startup and tools always show the real machine, so the demo set leaves them out.
const pages = (args.includes("--pages") ? args[args.indexOf("--pages") + 1] : demo ? "overview,explorer,junk,projects" : "overview,explorer,junk,projects,large,apps,tools,settings").split(",");
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
// Right after launch the page can still reload once, which destroys the context being evaluated.
for (let i = 0; ; i++) {
  try {
    if (await evaluate(`(async () => { for (let i = 0; i < 50 && !window.__reclaim?.getState().ready; i++) await new Promise(r => setTimeout(r, 200)); return !!window.__reclaim?.getState().ready })()`)) break;
  } catch (e) {
    if (i >= 20) throw e;
  }
  await sleep(1000);
}
if (lang) await evaluate(`window.__reclaimSetLang(${JSON.stringify(lang)})`);
if (demo) {
  const result = await evaluate(`(async () => {
    const root = ${JSON.stringify(demoRoot)}.toLowerCase().replace(/\\\\+$/, "") + "\\\\";
    const inside = (p) => String(p ?? "").toLowerCase().startsWith(root);
    const g = window.__reclaim.getState;
    for (let i = 0; i < 100 && !g().ready; i++) await new Promise((r) => setTimeout(r, 200));
    if (!inside(g().info?.userProfile)) return "the app is not running on the demo profile (USERPROFILE is " + g().info?.userProfile + ")";
    await g().scanExplorer(g().info.userProfile);
    await g().scanAll();
    const sys = (g().drives.find((d) => d.system)?.root ?? "C:\\\\").toLowerCase() + "windows\\\\";
    const own = (p) => inside(p) || String(p).toLowerCase().startsWith(sys);
    window.__reclaim.setState({
      junk: g().junk.filter((r) => r.found.every((f) => own(f.path))),
      installers: g().installers.filter((f) => inside(f.path)),
      projects: g().projects.filter((p) => inside(p.path)),
      drives: g().drives.filter((d) => !d.removable),
      info: { ...g().info, suggestedRoots: g().info.suggestedRoots.filter(inside) },
    });
    return "ok";
  })()`);
  if (result !== "ok") throw new Error(result);
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
