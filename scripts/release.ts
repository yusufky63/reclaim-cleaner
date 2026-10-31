// Prepares a release from the "Unreleased" notes in CHANGELOG.md and the Conventional Commits
// since the last tag. The Release PR workflow runs it after every push to main; it also works
// by hand.
//
//   node scripts/release.ts plan            prints the next version, or nothing to release
//   node scripts/release.ts apply <x.y.z>   sets every version field and moves the notes under it
//   node scripts/release.ts notes <x.y.z>   prints the notes of one version

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

/** Workspace crates whose version Cargo.lock records. */
const CRATES = ["reclaim-cleaner", "reclaim-core"];

export type Bump = "major" | "minor" | "patch";

const BREAKING = /^\w+(\([^)]*\))?!:|^BREAKING[ -]CHANGE:/m;
const FEATURE = /^feat(\([^)]*\))?!?:/;

/** A breaking change is major, a `feat` minor, anything else a patch. */
export function bumpFor(messages: string[]): Bump {
  if (messages.some((m) => BREAKING.test(m))) return "major";
  return messages.some((m) => FEATURE.test(m)) ? "minor" : "patch";
}

/** Before 1.0 a breaking change only raises the minor version, as SemVer allows. */
export function nextVersion(current: string, bump: Bump): string {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(current);
  if (!m) throw new Error(`not a plain x.y.z version: ${current}`);
  const [major, minor, patch] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (bump === "major" && major > 0) return `${major + 1}.0.0`;
  if (bump === "patch") return `${major}.${minor}.${patch + 1}`;
  return `${major}.${minor + 1}.0`;
}

/** Start and end of the body below `## [name]`: up to the next heading or the link list. */
function section(changelog: string, name: string): { start: number; end: number } | null {
  const heading = changelog.indexOf(`## [${name}]`);
  if (heading < 0) return null;
  const start = changelog.indexOf("\n", heading) + 1;
  const ends = [changelog.indexOf("\n## [", start), changelog.indexOf("\n[Unreleased]:", start)].filter((i) => i >= 0);
  return { start, end: ends.length ? Math.min(...ends) + 1 : changelog.length };
}

export function notesFor(changelog: string, name: string): string {
  const s = section(changelog, name);
  return s ? changelog.slice(s.start, s.end).trim() : "";
}

/** Moves the "Unreleased" notes under `## [version] - date` and adds its compare link. */
export function cutChangelog(changelog: string, version: string, date: string): string {
  const s = section(changelog, "Unreleased");
  const notes = notesFor(changelog, "Unreleased");
  if (!s || !notes) throw new Error("CHANGELOG.md has no Unreleased notes");
  let out = `${changelog.slice(0, s.start)}\n## [${version}] - ${date}\n\n${notes}\n\n${changelog.slice(s.end)}`;
  const link = /^\[Unreleased\]: (\S+)\/compare\/(\S+)\.\.\.HEAD$/m.exec(out);
  if (!link) throw new Error("CHANGELOG.md has no [Unreleased] compare link");
  const [line, base, previous] = link;
  out = out.replace(line, `[Unreleased]: ${base}/compare/v${version}...HEAD\n[${version}]: ${base}/compare/${previous}...v${version}`);
  return out;
}

function replaceOnce(text: string, pattern: RegExp, value: string, what: string): string {
  if (!pattern.test(text)) throw new Error(`no version found in ${what}`);
  return text.replace(pattern, `$1${value}$2`);
}

export function setPackageVersion(json: string, version: string): string {
  return replaceOnce(json, /^(\s*"version": ")[^"]+(")/m, version, "package.json");
}

export function setCargoVersion(toml: string, version: string): string {
  return replaceOnce(toml, /(\[workspace\.package\][^[]*?\nversion = ")[^"]+(")/, version, "Cargo.toml");
}

export function setLockVersions(lock: string, version: string): string {
  return CRATES.reduce(
    (text, name) => replaceOnce(text, new RegExp(`(name = "${name}"\\nversion = ")[^"]+(")`), version, `Cargo.lock (${name})`),
    lock,
  );
}

/** The version the bug report form suggests. */
export function setIssueVersion(yaml: string, version: string): string {
  return replaceOnce(yaml, /(label: ReClaim version[\s\S]*?placeholder: )\S+()/, version, "the bug report form");
}

const FILES = {
  changelog: "CHANGELOG.md",
  package: "package.json",
  cargo: "Cargo.toml",
  lock: "Cargo.lock",
  issue: ".github/ISSUE_TEMPLATE/bug_report.yml",
};

function edit(path: string, change: (text: string) => string) {
  writeFileSync(path, change(readFileSync(path, "utf8")));
}

function git(...args: string[]): string {
  return execFileSync("git", args, { encoding: "utf8" }).trim();
}

function tagExists(tag: string): boolean {
  try {
    git("rev-parse", "-q", "--verify", `refs/tags/${tag}`);
    return true;
  } catch {
    return false;
  }
}

function plan(): string {
  if (!notesFor(readFileSync(FILES.changelog, "utf8"), "Unreleased")) return "";
  const current = (JSON.parse(readFileSync(FILES.package, "utf8")) as { version: string }).version;
  const since = tagExists(`v${current}`) ? `v${current}` : git("describe", "--tags", "--abbrev=0", "--match", "v*");
  const messages = git("log", "--format=%B%x1e", `${since}..HEAD`)
    .split("\x1e")
    .map((m) => m.trim())
    .filter(Boolean);
  return nextVersion(current, bumpFor(messages));
}

function apply(version: string) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`not a plain x.y.z version: ${version}`);
  const date = new Date().toISOString().slice(0, 10);
  edit(FILES.changelog, (t) => cutChangelog(t, version, date));
  edit(FILES.package, (t) => setPackageVersion(t, version));
  edit(FILES.cargo, (t) => setCargoVersion(t, version));
  edit(FILES.lock, (t) => setLockVersions(t, version));
  edit(FILES.issue, (t) => setIssueVersion(t, version));
}

function main([command, version]: string[]) {
  if (command === "plan") {
    const next = plan();
    if (next) console.log(next);
  } else if (command === "apply" && version) {
    apply(version);
  } else if (command === "notes" && version) {
    console.log(notesFor(readFileSync(FILES.changelog, "utf8"), version));
  } else {
    throw new Error("usage: node scripts/release.ts plan | apply <x.y.z> | notes <x.y.z>");
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2));
}
