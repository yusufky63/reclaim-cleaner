// Builds a neutral demo profile for README screenshots, so no screenshot shows a real PC.
//
//   node scripts/demo-profile.mjs [C:\ReClaimDemo]          create it
//   node scripts/demo-profile.mjs [C:\ReClaimDemo] --remove remove it again
//
// Every file is created sparse (scripts/demo-sparse.ps1): sizes look real, disk use stays near
// zero. The tree holds browser and package caches, temp files, crash dumps, media, downloads and
// five projects with build output, each with its own last activity. Start the app with this
// profile as shown at the end, then run `node scripts/screenshots.mjs --demo`.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
const root = args.find((a) => !a.startsWith("--")) ?? "C:\\ReClaimDemo";
const MARKER = ".reclaim-demo";

if (args.includes("--remove")) {
  // Only a folder this script created is removed, never an arbitrary path.
  if (!existsSync(join(root, MARKER))) throw new Error(`${root} is not a ReClaim demo profile`);
  rmSync(root, { recursive: true, force: true });
  console.log(`removed ${root}`);
  process.exit(0);
}
if (existsSync(root)) throw new Error(`${root} already exists; remove it first with --remove`);

const R = `${root}\\Users\\Alex`;
const LOCAL = `${R}\\AppData\\Local`;
const ROAM = `${R}\\AppData\\Roaming`;
const TEMP = `${LOCAL}\\Temp`;

// Fixed seed: the same profile, and so the same screenshots, every time.
let seed = 20261016;
const rand = () => {
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};
const hex = (n) => Array.from({ length: n }, () => Math.floor(rand() * 16).toString(16)).join("");
const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;
const DAY = 86400_000;
const now = Date.now();

const files = [];
const dirTimes = [];
const add = (path, bytes, daysAgo = 2) => files.push([path, Math.round(bytes), now - daysAgo * DAY - Math.floor(rand() * 3600_000)]);

/** `count` files in `dir` whose sizes add up to about `total`, skewed like real caches. */
function spread(dir, count, total, name, daysAgo = [1, 20]) {
  const weights = Array.from({ length: count }, () => Math.pow(rand(), 3) + 0.02);
  const sum = weights.reduce((a, b) => a + b, 0);
  weights.forEach((w, i) => add(`${dir}\\${name(i)}`, (w / sum) * total, daysAgo[0] + rand() * (daysAgo[1] - daysAgo[0])));
}

// Caches and temp files the junk rules find.
spread(TEMP, 70, 260 * MB, () => `tmp${hex(4).toUpperCase()}.tmp`, [3, 40]);
add(`${TEMP}\\vs_setup_cache.bin`, 380 * MB, 21);
add(`${TEMP}\\node-v22-download.zip`, 31 * MB, 12);
for (const profile of ["Default", "Profile 1"]) {
  const base = `${LOCAL}\\Google\\Chrome\\User Data\\${profile}`;
  spread(`${base}\\Cache\\Cache_Data`, profile === "Default" ? 320 : 120, profile === "Default" ? 640 * MB : 210 * MB, () => `f_${hex(6)}`);
  spread(`${base}\\Code Cache\\js`, 110, 150 * MB, () => `${hex(16)}_0`);
  spread(`${base}\\GPUCache`, 4, 14 * MB, (i) => ["data_0", "data_1", "data_2", "index"][i]);
}
spread(`${LOCAL}\\Microsoft\\Edge\\User Data\\Default\\Cache\\Cache_Data`, 160, 280 * MB, () => `f_${hex(6)}`);
spread(`${LOCAL}\\Microsoft\\Edge\\User Data\\Default\\Code Cache\\js`, 60, 70 * MB, () => `${hex(16)}_0`);
spread(`${LOCAL}\\Mozilla\\Firefox\\Profiles\\k2x7m4qa.default-release\\cache2\\entries`, 210, 330 * MB, () => hex(40).toUpperCase());
for (let i = 0; i < 420; i++) {
  const h = hex(124);
  add(`${LOCAL}\\npm-cache\\_cacache\\content-v2\\sha512\\${h.slice(0, 2)}\\${h.slice(2, 4)}\\${h.slice(4)}`, Math.pow(rand(), 4) * 18 * MB + 2 * KB, 5 + rand() * 90);
}
spread(`${LOCAL}\\npm-cache\\_logs`, 12, 3 * MB, (i) => `2026-0${1 + (i % 9)}-1${i % 10}T10_2${i % 6}_00_000Z-debug-0.log`, [10, 60]);
spread(`${LOCAL}\\pip\\Cache\\http-v2\\a\\b`, 160, 440 * MB, () => hex(56), [8, 120]);
spread(`${ROAM}\\Code\\Cache\\Cache_Data`, 90, 120 * MB, () => `f_${hex(6)}`);
spread(`${ROAM}\\Code\\CachedData\\${hex(40)}\\chrome\\js`, 40, 260 * MB, () => `${hex(16)}_0`, [15, 40]);
spread(`${ROAM}\\Code\\logs\\20260${3 + Math.floor(rand() * 6)}10T091500`, 18, 42 * MB, (i) => `window${i}\\renderer.log`);
spread(`${ROAM}\\discord\\Cache\\Cache_Data`, 140, 210 * MB, () => `f_${hex(6)}`);
spread(`${ROAM}\\discord\\Code Cache\\js`, 40, 55 * MB, () => `${hex(16)}_0`);
spread(`${ROAM}\\Slack\\Cache\\Cache_Data`, 70, 95 * MB, () => `f_${hex(6)}`);
spread(`${LOCAL}\\Spotify\\Data`, 45, 1.1 * GB, () => `${hex(2)}\\${hex(40)}.file`);
add(`${LOCAL}\\CrashDumps\\Code.exe.18244.dmp`, 182 * MB, 26);
add(`${LOCAL}\\CrashDumps\\explorer.exe.9120.dmp`, 64 * MB, 48);
spread(`${R}\\.gradle\\wrapper\\dists\\gradle-8.7-bin\\${hex(25)}`, 2, 132 * MB, (i) => ["gradle-8.7-bin.zip", "gradle-8.7-bin.zip.ok"][i], [60, 90]);
spread(`${R}\\.gradle\\caches\\modules-2\\files-2.1`, 260, 940 * MB, () => `${hex(8)}\\${hex(40)}.jar`, [20, 200]);
spread(`${R}\\.cargo\\registry\\cache\\index.crates.io-1949cf8c6b5b557f`, 220, 260 * MB, (i) => `crate-${i}-1.${i % 20}.${i % 7}.crate`, [5, 150]);
spread(`${R}\\.cargo\\registry\\src\\index.crates.io-1949cf8c6b5b557f`, 300, 780 * MB, (i) => `crate-${i % 110}-1.${i % 20}.0\\src\\${["lib", "mod", "parse", "util"][i % 4]}.rs`, [5, 150]);

// The demo user's own files, for the space map and large files.
add(`${R}\\Videos\\Family trip 2025.mp4`, 4.2 * GB, 210);
add(`${R}\\Videos\\Screen recording 2026-03-02.mkv`, 1.6 * GB, 228);
add(`${R}\\Videos\\Drone\\DJI_0042.MP4`, 2.1 * GB, 160);
add(`${R}\\Videos\\Drone\\DJI_0043.MP4`, 1.8 * GB, 160);
spread(`${R}\\Pictures\\Camera Roll`, 380, 2.1 * GB, (i) => `IMG_2025${String(1 + (i % 12)).padStart(2, "0")}${String(1 + (i % 28)).padStart(2, "0")}_${String(i).padStart(4, "0")}.jpg`, [40, 400]);
spread(`${R}\\Pictures\\Screenshots`, 120, 150 * MB, (i) => `Screenshot ${i + 1}.png`, [5, 200]);
spread(`${R}\\Music\\Library`, 210, 1.6 * GB, (i) => `Track ${String(i + 1).padStart(3, "0")}.mp3`, [300, 900]);
spread(`${R}\\Documents\\Invoices`, 40, 38 * MB, (i) => `Invoice-${2024 + (i % 3)}-${String(i + 1).padStart(3, "0")}.pdf`, [30, 700]);
add(`${R}\\Documents\\Thesis draft.docx`, 12 * MB, 33);
add(`${R}\\Documents\\Budget 2026.xlsx`, 2 * MB, 8);
add(`${R}\\Documents\\Backups\\phone-backup-2025.zip`, 6.5 * GB, 320);
add(`${R}\\Downloads\\ubuntu-24.04.2-desktop-amd64.iso`, 6.0 * GB, 140);
add(`${R}\\Downloads\\VSCodeUserSetup-x64.exe`, 98 * MB, 95);
add(`${R}\\Downloads\\node-v22-x64.msi`, 30 * MB, 120);
add(`${R}\\Downloads\\dataset.csv`, 820 * MB, 64);
add(`${R}\\Downloads\\Photos.zip`, 1.2 * GB, 90);
add(`${R}\\Downloads\\Photos (1).zip`, 1.2 * GB, 89);

// Projects with build output. Folder times matter too: a project's last activity comes from them.
const P = `${R}\\Projects`;
function project(name, days, sources, artifacts) {
  for (const [rel, size] of sources) add(`${P}\\${name}\\${rel}`, size, days);
  for (const [rel, count, total, ext] of artifacts) spread(`${P}\\${name}\\${rel}`, count, total, () => `${hex(2)}\\${hex(12)}${ext ?? ""}`, [days, days + 5]);
  dirTimes.push([`${P}\\${name}`, now - days * DAY]);
}
const pkg = (n) => Array.from({ length: n }, (_, i) => [`node_modules\\pkg-${i}\\package.json`, 2 * KB]);
project("web-shop", 52, [["package.json", 2 * KB], ["next.config.mjs", 1 * KB], ["src\\app\\page.tsx", 6 * KB], ["src\\app\\layout.tsx", 3 * KB], ...pkg(3)], [
  ["node_modules", 520, 610 * MB, ".js"],
  [".next\\cache", 180, 450 * MB],
  [".git\\objects\\pack", 3, 120 * MB, ".pack"],
]);
project("api-server", 3, [["Cargo.toml", 1 * KB], ["src\\main.rs", 9 * KB], ["src\\routes.rs", 14 * KB]], [
  ["target\\debug\\deps", 320, 2.1 * GB, ".rlib"],
  ["target\\release\\deps", 140, 780 * MB, ".rlib"],
]);
project("ml-notebook", 120, [["requirements.txt", 1 * KB], ["train.py", 12 * KB], ["notebooks\\explore.ipynb", 840 * KB], ["data\\features.parquet", 2.3 * GB]], [
  ["__pycache__", 6, 2 * MB, ".pyc"],
  [".venv\\Lib\\site-packages", 380, 1.4 * GB, ".pyd"],
]);
project("mobile-app", 76, [["package.json", 2 * KB], ["app.json", 1 * KB], ["android\\build.gradle", 3 * KB], ["android\\app\\build.gradle", 4 * KB], ...pkg(2)], [
  ["node_modules", 410, 480 * MB, ".js"],
  ["android\\app\\build", 160, 690 * MB, ".dex"],
  ["android\\.gradle", 60, 250 * MB, ".bin"],
]);
project("docs-site", 10, [["package.json", 2 * KB], ["astro.config.mjs", 1 * KB], ["src\\pages\\index.astro", 4 * KB], ...pkg(2)], [
  ["node_modules", 300, 350 * MB, ".js"],
  ["dist", 40, 31 * MB, ".html"],
]);

mkdirSync(root, { recursive: true });
writeFileSync(join(root, MARKER), "Created by scripts/demo-profile.mjs. Safe to delete.\n");
const manifest = join(tmpdir(), `reclaim-demo-${process.pid}.json`);
writeFileSync(manifest, JSON.stringify({ files, dirTimes }));
const ps1 = join(dirname(fileURLToPath(import.meta.url)), "demo-sparse.ps1");
try {
  execFileSync("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1, "-Manifest", manifest], { stdio: "inherit" });
} finally {
  rmSync(manifest, { force: true });
}

// Settings the app reads from the demo APPDATA: English, onboarding done, demo projects only.
const settingsDir = `${ROAM}\\ReClaim`;
mkdirSync(settingsDir, { recursive: true });
writeFileSync(join(settingsDir, "settings.json"), JSON.stringify({ language: "en", theme: "system", projectRoots: [P], onboarded: true }, null, 2));

const total = files.reduce((s, f) => s + f[1], 0);
console.log(`${files.length} files, ${(total / GB).toFixed(1)} GB logical, in ${root}`);
console.log(`
Start the app on this profile (PowerShell, after "cargo build -p reclaim-cleaner" and with "pnpm dev" running):
  $env:USERPROFILE="${R}"; $env:LOCALAPPDATA="${LOCAL}"; $env:APPDATA="${ROAM}"; $env:TEMP="${TEMP}"; $env:TMP="${TEMP}"; $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333"; .\\target\\debug\\reclaim-cleaner.exe
Then: node scripts/screenshots.mjs --demo`);
