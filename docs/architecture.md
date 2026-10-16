# Architecture

ReClaim Cleaner is a Windows 10/11 disk cleaner and space analyzer. It is a Tauri 2 desktop app: a Rust engine does every file system, registry and process operation, and a React UI renders the results. This document is the plan the code follows. The prior-art review behind these decisions lives in [research.md](research.md).

## Goals

1. Show where the space went, in one screen, in under a minute.
2. Free space without surprises: every delete runs from a reviewed plan, and nothing outside known, regenerable locations is preselected.
3. Stay small and honest: a ~10 MB app, no telemetry, no bundled offers, no registry cleaner, no nag screens.
4. Be easy to extend: cleanup targets are data (JSON rules), not code.

## Non-goals

- Registry cleaning, "debloating", service or privacy tweaks. Microsoft does not support registry cleaners and the risk is not worth the bytes.
- File shredding or free-space wiping. On SSDs it cannot be promised (wear leveling, TRIM).
- Background monitoring or scheduled deletion.
- Deleting inside `WinSxS`, `C:\Windows\Installer`, the reset ESD files or `hiberfil.sys` directly. System areas are changed only through Windows' own tools (DISM, `powercfg`, the Delivery Optimization cmdlet, `cleanmgr`).

## Repository layout

```
crates/core/        reclaim-core: the engine. Pure Rust, no Tauri dependency, unit tested.
  src/paths.rs        env var + wildcard expansion, protected paths, path guards
  src/fsmeta.rs       attributes: reparse points, cloud placeholders, age
  src/walk.rs         parallel size walker (rayon over std::fs::read_dir)
  src/explorer.rs     arena tree for the treemap, large files, extension stats
  src/rules.rs        rule schema + embedded catalog (rules/*.json)
  src/junk.rs         scan cleanup targets
  src/artifacts.rs    find build artifacts in project folders
  src/clean.rs        execute a reviewed plan (recycle or permanent)
  src/categories.rs   treemap categories from folder names and extensions
  src/duplicates.rs   size -> partial hash -> full hash pipeline
  src/empty.rs        empty folder finder
  src/installers.rs   old installers and archives in Downloads and on the Desktop
  src/apps.rs         installed programs (registry) + Store apps
  src/startup.rs      Run keys and Startup folders, StartupApproved toggles
  src/system.rs       drives, elevation, processes, Recycle Bin API, Windows version
  src/hidden.rs       hidden space: shadow storage (WMI), reserve, MFT, system files, virtual disks
  src/locks.rs        what is using a file or folder (Restart Manager), graceful close
  src/shell.rs        Explorer context menu verbs and command-line paths
  src/steam.rs        Steam folder, libraries, manifests and shader caches
  src/settings.rs     settings + exclusions, portable mode data folder
  src/history.rs      JSON Lines cleanup log
src-tauri/          the desktop shell: Tauri commands, events, elevation helper
rules/              targets.json (fixed locations) and artifacts.json (project artifacts)
src/                React UI (Vite, Tailwind v4, i18next, zustand)
docs/               architecture, rules format, research, FAQ
.github/            CI, release, issue forms, dependabot
```

The engine is a separate crate so `cargo test` runs without a WebView, and so a future CLI can reuse it.

## Data flow

```
UI (React) --invoke--> Tauri command --> reclaim-core (rayon threads)
UI <--event "progress"-- progress pump (every 150 ms) <-- atomic counters
UI <--result------------ command returns serialized result
```

Long jobs (explorer scan, junk scan, duplicate scan, artifact scan) run on a blocking thread. Each job has a `CancelToken` (an `AtomicBool`) and a `Progress` struct of atomic counters. A pump thread emits `job://progress` events until the job ends, so the UI never polls.

The explorer keeps its tree in Rust state (an arena of nodes). The UI asks for one node and two levels of children at a time, so a scan of a full drive never crosses the IPC bridge in one piece.

## Safety model

These rules are enforced in the engine, not only in the UI.

1. **Preview first.** Scans never delete. A clean takes a plan the user has seen: rule id, path, size, risk.
2. **Re-validate before deleting.** Each path is checked again right before deletion: it must still exist, must not be a reparse point, must sit under a root the rule allows, and must not hit the protected list or a user exclusion.
3. **Protected list (cannot be overridden).** Drive roots, `%WINDIR%` itself, `System32`, `SysWOW64`, `WinSxS`, `Windows\Installer`, `Program Files` roots, `ProgramData` root, the user profile root, user libraries (Desktop, Documents, Downloads, Pictures, Music, Videos), OneDrive roots, `.git` folders and the app's own data folder. Rules may target folders below some of these (for example `%WINDIR%\Temp`), never the folders themselves.
4. **Never follow junctions or symlinks.** The walker checks `FILE_ATTRIBUTE_REPARSE_POINT` and does not descend. Deletion uses our own walker with the same check, so a junction inside a cache cannot redirect a delete.
5. **Cloud placeholders are not read.** Files with `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN` or `OFFLINE` are skipped by the duplicate hasher and are not counted as used space.
6. **Running apps.** A rule can name processes (`chrome.exe`). While one is running, the rule is shown as blocked and is not cleaned. We never kill processes.
7. **Risk levels.** `safe` items (regenerable caches) are preselected. `review` items are visible but unselected. `caution` items are hidden until "Show caution items" is turned on in Settings, and each shows its warning.
8. **Delete method.** Caches, temp files and project build folders are deleted permanently by default, because moving 10 GB of cache or `node_modules` into the Recycle Bin frees nothing. Your own files (large files, duplicates, empty folders, old installers) go to the Recycle Bin by default. Both defaults are in Settings and every confirm dialog states the method and lets you switch it.
9. **Least privilege.** The app runs as the current user. Rules that need admin rights are cleaned by the same exe started elevated in helper mode (`--elevated-clean`). The helper receives only rule ids and options, never paths, and expands the paths itself, so a tampered plan file cannot point it anywhere else.
10. **Everything is logged** to `history.jsonl`: time, kind, rule ids, item count, bytes freed, method, errors.
11. **Command-line paths are untrusted.** `--analyze <folder>` may come from Explorer, another program, or (forwarded by the running copy) any process on the desktop. Paths must be existing folders on a drive letter, and they only choose what to map; nothing is ever deleted from a command line.
12. **Processes are asked, never forced.** "What is using this?" only offers the request a window's close button sends, re-checks that the process id still belongs to the same process, and never ends processes or closes their handles.

## Rules

Rules are JSON, embedded at build time, and documented in [rules.md](rules.md).

- `rules/targets.json`: fixed locations (temp, caches, logs, dumps). Fields: `id`, `group`, `name`/`nameTr`, `description`/`descriptionTr`, `paths` (env vars + `*` inside one segment), `mode` (`contents`, `folder`, `files`, `report`), `include` globs for `files`, `minAgeHours`, `risk`, `admin`, `closeApps`, `command`, `verified`, `source`.
- `rules/artifacts.json`: project build folders. Fields: `id`, `dirName` (glob), `markers` (any one must exist in the parent), `contains` (must exist inside), `risk`, descriptions.

`report` rules are never deleted as files. They show the size and offer the official command (for example `Delete-DeliveryOptimizationCache`, `DISM /StartComponentCleanup`, `cleanmgr /autoclean` for Windows.old).

## Explorer and categories

The walker builds a tree of folders. Files of 4 MB or more are kept as their own nodes; smaller files are folded into one "small files" bucket per folder. Every node gets a category from its path and extension:

| Slot | Category | Examples |
|---|---|---|
| 1 | Projects and code | folders with `package.json`, `Cargo.toml`, `.git`, `node_modules`, `target` |
| 2 | Documents and media | Documents, Pictures, Videos, Music, office files, images, video |
| 3 | Caches and temp | `AppData\Local\*\Cache`, `Temp`, package manager caches |
| 4 | Apps and games | Program Files, Steam libraries, `AppData\Local\Programs` |
| 5 | AI models | `.ollama`, Hugging Face cache, LM Studio, `.safetensors`, `.gguf` |
| 6 | Windows and system | `Windows`, `ProgramData`, `pagefile.sys`, `hiberfil.sys` |
| - | Other | neutral grey |

The six colors are the validated categorical palette from the dataviz method, in fixed order. Light mode has three slots under 3:1 contrast, so every tile carries a text label and the treemap has a list view beside it.

The "Cleanup" view switches tiles to neutral and paints only reclaimable space, hatched, in the status colors (green safe, amber review), with a legend.

## Elevation

- The app manifest stays `asInvoker`. No UAC prompt on launch, drag and drop keeps working.
- "Restart as administrator" relaunches with `ShellExecuteExW("runas")`.
- Admin rules in a normal session go through helper mode: one UAC prompt per clean, the helper writes a result JSON, the UI reads it.
- `--elevated-measure` reads shadow storage sizes through WMI (`Win32_ShadowStorage`, `Win32_ShadowCopy`), which Windows only answers for administrators. It takes no input from the request and changes nothing.
- Explorer integration is a classic per-user verb under `HKCU\Software\Classes` (`Directory`, `Directory\Background`, `Drive`), written and removed by the app. Explorer starts a new process per selected folder; the single-instance plugin forwards each path to the running copy, which merges a burst into one map. When that copy runs as administrator, it lets `WM_COPYDATA` from standard-user processes reach the plugin's window (`ChangeWindowMessageFilterEx`), otherwise the path would be dropped. The NSIS uninstaller removes the verbs, except when a setup replaces the installed copy (it runs the uninstaller in place, `_?=`, while a real uninstall runs from a copy in TEMP).

## Portable mode

If a file named `portable` sits next to the exe, settings and history live in `.\data` beside it instead of `%APPDATA%\ReClaim`. The portable zip ships with that file.

The NSIS uninstaller's "Delete the application data" box removes `%APPDATA%\ReClaim` (from `windows/hooks.nsh`) along with Tauri's own `%APPDATA%` and `%LOCALAPPDATA%` folders named after the bundle identifier. It is unticked by default, never shown in silent or passive runs and ignored with `/UPDATE`, so replacing the installed copy keeps the data. The MSI and the portable build never delete it.

## Packaging

- `pnpm tauri build` produces an NSIS setup (`currentUser`, English and Turkish, WebView2 bootstrapper embedded) and MSI packages (en-US, tr-TR).
- The release workflow zips the raw exe with the `portable` marker as `ReClaim-Cleaner_<version>_x64_portable.zip` and publishes SHA-256 checksums.
- Updates are checked on request (or at start, if enabled) against the GitHub Releases API; the app opens the release page and never installs on its own.
- Release assets are named without spaces (`ReClaim-Cleaner_<version>_x64-setup.exe`) so download URLs stay stable.

## Internationalization

UI strings live in `src/i18n/en.ts` and `src/i18n/tr.ts`; the Turkish file is typed against the English one, so a missing key fails the typecheck. Rule names and descriptions carry both languages in the rule files. Numbers, sizes and dates use `Intl` with the active locale.

## Testing

- `crates/core`: unit tests for path expansion, protected paths, wildcard matching, rule validation (every shipped rule must parse and pass the guards), walker behavior on junctions, duplicate grouping, empty folder detection, artifact detection and clean plan validation, using temp folders.
- Windows-facing parts are tested against the real system where it is safe: the Restart Manager finds the test process holding a temp file, registry verbs are written under a throwaway `HKCU\Software\ReClaimTest-*` key and removed, and Steam libraries, manifests and shader caches are built in temp folders.
- UI: Vitest for the treemap layout, byte formatting, script generation, selection helpers, hidden space arithmetic and the grouping of processes.
- CI on `windows-latest`: typecheck, ESLint (`--max-warnings=0`), Vitest, build, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.
