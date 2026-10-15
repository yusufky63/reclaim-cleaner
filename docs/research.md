# Research: open-source landscape for a Windows disk cleaner

Scope: prior art for a Windows 10/11 disk cleaner and space analyzer built with Tauri 2, a Rust backend and a React UI, released under MIT. Planned features: an overview dashboard, a treemap explorer, a rule-based junk cleaner, a developer artifact cleaner, a large file finder, a duplicate finder, an empty folder finder, an uninstaller with leftover scan, a startup manager, cleanup history, a PowerShell script export, and a Turkish and English UI.

Research date: 2026-10-16. Sources are project READMEs, official docs, Microsoft Learn and press coverage, all fetched on that date. Star counts are rounded figures from the GitHub pages on that date and will drift.

Markers used in this document:

- **(verified)**: confirmed in a primary source (project README, project docs or Microsoft Learn).
- **(secondary)**: from press, third-party guides or wikis.
- **(unverified)**: from general knowledge or a source I could not confirm. Check it before relying on it.

---

## 1. Summary

1. **Cleaning itself is not hard. Deciding what is safe to delete is.** Every mature project puts its effort into guardrails: preview before delete (BleachBit, Mole, npkill, cargo-sweep), trash or recycle bin as a first-class delete action (dua, gdu, Krokiet), allowlists (BleachBit, Mole, Czkawka), protected options behind an "expert mode" (BleachBit 5.1+), confidence levels for leftovers (BCU), and reference folders that can never be deleted (Czkawka).
2. **Rules should be data, not code.** BleachBit's CleanerML, Winapp2.ini and Windows' own `VolumeCaches` registry handlers all describe cleanup targets declaratively: a base folder, file patterns, recursion flags, a minimum age, a "running process" guard, a warning text and a localized label. Our rule format should combine the best parts of these (section 9, item P0-3).
3. **Scanning speed comes from three techniques.** Parallel directory walking (dust, dua, gdu, pdu, diskus). Reading the NTFS MFT directly when elevated (WizTree, TreeSize, WinDirStat 2.x). Caching results between runs (Czkawka hash cache, ncdu and gdu exports, QDirStat cache files). On Windows, Rust's `DirEntry::metadata()` costs no extra system call, so a plain parallel walk is already competitive (verified, Rust std docs).
4. **Size accounting has known traps.** Hard links (WinSxS is mostly hard links, so Explorer overstates it), compressed and sparse files (logical size vs on-disk size), reparse points (junctions and symlinks cause double counting and loops), and cloud placeholders (OneDrive "online-only" files: reading their content downloads them).
5. **Developer artifact cleaning is a solved pattern.** Detect a project by marker files (`Cargo.toml`, `package.json` and others), list the known artifact folders for that project type, and use "last modified" so recently active projects are not preselected (kondo, npkill, Mole purge, dua clean, cargo-sweep).
6. **Uninstalling is where users get hurt.** BCU shows the right pattern: run the vendor uninstaller first, scan leftovers afterwards, score each leftover by confidence, preselect only high-confidence items, and offer a backup.
7. **What not to ship:** a registry cleaner (Microsoft does not support them), "system slimming" that deletes Windows components, debloat or telemetry tweaks, disabling Defender or Windows Update, bundled offers, nag screens, always-on monitoring and telemetry. CCleaner's history (2017 supply-chain compromise, 2018 Active Monitoring backlash, 2020 Defender PUA flag for bundling) shows what destroys trust.
8. **Use Windows' own servicing tools for system areas.** Component store: DISM `/StartComponentCleanup`. Delivery Optimization: `Delete-DeliveryOptimizationCache`. Hibernation: `powercfg /hibernate`. Previous installations: the Settings or cleanmgr route. Never delete `WinSxS`, `C:\Windows\Installer` or the ESD reset files by hand.
9. **License hygiene matters for an MIT project.** BleachBit, WinDirStat, QDirStat, K4DirStat, Mole, Optimizer and Krokiet are GPL. Winapp2.ini is CC-BY-SA-4.0. We can learn from their ideas and behavior but must not copy their code or rule files. Czkawka core, dua, dust, gdu, kondo, npkill, diskus, pdu, BCU (Apache-2.0), Win11Debloat and Sophia Script use permissive licenses. Even with those, keep attribution if code is reused.

---

## 2. Projects reviewed

| Project | Link | License | Stars (approx.) | What to borrow |
|---|---|---|---|---|
| BleachBit | https://github.com/bleachbit/bleachbit | GPL-3.0-or-later | 7.1k | CleanerML rule model (search modes, regex filters, `running` guard, OS scoping, warnings), preview-then-delete, allowlist, expert mode for protected options |
| Winapp2.ini | https://github.com/MoscaDotTo/Winapp2 | CC-BY-SA-4.0 | 1.1k | Community rule database idea, explicit paths instead of broad patterns, separate "aggressive" file (Winapp3) kept apart. Do not bundle it in an MIT app without legal review |
| Czkawka / Krokiet | https://github.com/qarmin/czkawka | MIT (core, CLI); Krokiet GPL-3.0-only | 34.3k | Duplicate pipeline (size, then first/last 4 KB prehash, then full hash), BLAKE3/xxh3, hash cache, reference folders, excluded globs, empty-folder propagation, temp-file patterns, selection helpers, presets |
| WinDirStat 2.x | https://github.com/windirstat/windirstat | GPL-3.0 | 4.3k | Treemap with extension stats, accelerated NTFS scan when elevated, duplicates off by default, cloud-file safeguards, logical vs physical size, hardlink display, custom cleanup commands |
| WizTree | https://diskanalyzer.com | Proprietary (free for personal use) | n/a | Direct MFT reading on NTFS for speed, counting hard-linked files once, CSV export |
| SpaceSniffer | http://www.uderzo.it (secondary) | Freeware, closed source | n/a | Compact filter syntax combining mask, size and age, color tags, portable with no registry writes (secondary) |
| QDirStat | https://github.com/shundhammer/qdirstat | GPL-2.0 | 2.6k | User-defined cleanup actions with an output window, file type stats with drill-down, file age stats by year and month, exclude rules, saved scan cache |
| K4DirStat | https://github.com/jeromerobert/k4dirstat | GPL-2.0 | 82 | Tree view plus treemap pairing (low activity project) |
| TreeSize Free | https://www.jam-software.com/treesize_free/ | Proprietary freeware | n/a | MFT scan with a threaded fallback, treemap, NTFS compression per branch (secondary) |
| dust | https://github.com/bootandy/dust | Apache-2.0 | 12.5k | Threaded walker, hard links counted once, on-disk vs apparent size switch, type grouping, JSON output, screen-reader mode (`-R`) |
| dua-cli | https://github.com/Byron/dua-cli | MIT | 6.3k | Parallel scan, multi-stage delete confirmation, trash or permanent delete as separate actions, `dua clean` that skips git-tracked content, localized UI |
| gdu | https://github.com/dundee/gdu | MIT | 6.1k | Parallel scan with a sequential mode for HDDs, hard links counted once and flagged, export/import JSON, `--no-delete` safety flags, delete blocked while filters are active |
| ncdu | https://dev.yorhel.nl/ncdu | MIT | n/a (not on GitHub) | Opt-in multithreaded scan (2.x), binary export for trees larger than memory |
| diskus | https://github.com/sharkdp/diskus | MIT or Apache-2.0 | 1.3k | Fast parallel total size; documents that Windows hard links and junctions are counted like Explorer does |
| pdu | https://github.com/KSXGitHub/parallel-disk-usage | Apache-2.0 | 739 | Optional hardlink dedup (with documented cost), JSON in/out |
| jwalk | https://github.com/Byron/jwalk | MIT | 288 | Parallel sorted walk on rayon. **Archived and unmaintained**, so do not depend on it |
| npkill | https://github.com/voidcosmos/npkill | MIT | 9.5k | Target profiles per ecosystem, documented risk notes per folder name, "last workspace usage", dry run, warning for node_modules belonging to installed apps |
| kondo | https://github.com/tbillington/kondo | MIT | 2.5k | Marker-file to artifact-dir table for 20+ project types, `--older` filter |
| cargo-sweep | https://github.com/holmgr/cargo-sweep | MIT | 1k | Time-based artifact removal, stamp file, `--maxsize`, dry run. Unmaintained |
| Mole | https://github.com/tw93/Mole | GPL-3.0 (per current LICENSE file) | 70.5k | Command set (clean, uninstall, optimize, analyze, status, purge, installer), `--dry-run` everywhere, operations log plus `history`, protected paths, elevation only when needed, fail closed if the allowlist cannot be read, recent artifacts unselected |
| Pearcleaner | https://github.com/alienator88/Pearcleaner | Apache-2.0 with Commons Clause | 14.9k | Uninstall with leftovers, orphaned file search, privileged helper only for system folders. Project is on hold |
| Bulk Crap Uninstaller | https://github.com/Klocman/Bulk-Crap-Uninstaller | Apache-2.0 | 21.9k | Detection sources (registry, Store, Steam, Chocolatey, orphans), quiet uninstall, leftover confidence levels, uninstaller certificate check, Program Files orphan cleanup, startup manager |
| Optimizer (hellzerg) | https://github.com/hellzerg/optimizer | GPL-3.0 | 18.2k | Little to borrow. Archived January 2026. Mixes cleaning with risky tweaks (Defender, updates, services) |
| Win11Debloat | https://github.com/Raphire/Win11Debloat | MIT | 59.3k | Clear opt-in menus, settings export/import, revertible changes. Out of scope for a cleaner |
| Sophia Script | https://github.com/farag2/Sophia-Script-for-Windows | MIT | 9.8k | One function per change with an explicit argument and a matching revert, preset file. Cleanup tasks for `SoftwareDistribution\Download` and temp folders |
| trash-rs | https://github.com/Byron/trash-rs | MIT | 255 | Rust crate for moving files to the Windows Recycle Bin. Used by dua and Czkawka |
| Windows built-ins | Microsoft Learn (see Sources) | n/a | n/a | VolumeCaches handler model, Storage Sense, Cleanup recommendations, DISM component cleanup, Delivery Optimization cmdlet, hibernation, Windows.old |
| CCleaner, Wise Disk Cleaner, PrivaZer | vendor sites | Proprietary | n/a | Feature ideas only (scheduled cleaning, per-app cache lists). Mostly lessons in what not to do |

---

## 3. Per-project notes

### 3.1 BleachBit (verified)

- **CleanerML.** An XML rule format validated by an XSD schema. A cleaner contains options, and each option contains actions. Rules are loaded from the app and from a user folder (`%APPDATA%\BleachBit\cleaners` on Windows).
- **Search modes.** `file` (one file), `glob`, `walk.files` (files only), `walk.all` (files and dirs, excluding the top dir), `walk.top` (including the top dir), and `deep` (scattered files such as `Thumbs.db`; deep scans of one directory are merged for speed).
- **Filters.** `regex`/`nregex` on the relative path, `wholeregex`/`nwholeregex` on the full path, and `type` (file or directory).
- **Commands.** A closed set: `delete`, `truncate`, shred, `winreg`, `ini`, `json`, `sqlite.vacuum`, and app-specific handlers. We should keep a closed set too, and leave out `winreg`.
- **Guards.** `running` aborts cleaning when a named exe (or path) is running. An `os` attribute scopes cleaners, actions and variables. Variables support env vars and multi-value expansion. Labels can be extracted for gettext.
- **UX.** Select options, click Preview, review the list, then Delete. "Confirm before delete" stays locked on unless expert mode is enabled.
- **Expert mode (5.1+, refined in 6.0, released 2026-04-25).** Protected options (browser passwords, wiping empty space, Explorer shellbags/recent docs/thumbnails, deep scan, some app-specific items) show a warning icon. They are skipped unless expert mode is on. With expert mode on, each protected option asks for confirmation, and you can tick "remember". There is a reset for remembered confirmations.
- **6.0 changes worth copying.**
  - "Whitelist" was renamed "allowlist".
  - Adding a protected path to custom paths now shows a warning.
  - A notice explains that elevating to another account only cleans that account's user data.
  - A recycle-bin bug was fixed: emptying the bin used to follow junctions and symlinks, which could delete unrelated data. **This is a key pitfall for us.**
- **Shredding.** BleachBit's own docs call shredding and free-space wiping on SSDs best effort only: wear leveling, TRIM and spare area defeat it, and NTFS can keep small files inside the MFT. For SSDs they recommend full-disk encryption or the vendor's secure erase instead. We should not ship shredding as a privacy promise (see section 7).
- **Updates.** The update check only notifies. It never downloads automatically.

### 3.2 Winapp2.ini (verified)

- A community database of explicit cleanup paths per application, consumed by CCleaner, BleachBit and others.
- It is shipped "as is" with a warning that deletions may be irreversible.
- Its most aggressive entries live in a separate file, Winapp3, with a stronger warning.
- License is CC-BY-SA-4.0. That is share-alike, so do not bundle it in an MIT app without legal review. An optional user-supplied import is the safer path (P2).

### 3.3 Czkawka and Krokiet (verified unless noted)

- **Tools.** Duplicates, empty folders, big files, empty files, temporary files, similar images, similar videos, same music, invalid symlinks, broken files and bad extensions. Krokiet (Slint) is the current GUI; the GTK GUI ended at 12.0.
- **Duplicate pipeline.**
  1. Group by size and drop sizes that occur only once.
  2. Prehash the first and last 4 KB.
  3. Full-hash only the files that survive.
  - Hash choices: BLAKE3 (recommended), xxh3 (fastest) and CRC32. Name-only and size-only modes are explicitly unreliable.
- **Caches.**
  - Hash and prehash caches are stored as bincode, with optional JSON copies.
  - A stopped scan keeps the hashes it already computed.
  - There is an option to evict outdated entries. Disable it for removable drives so their entries are not lost while the drive is disconnected.
  - The exact invalidation key is not documented. Path plus size plus modification time is a reasonable inference (unverified).
- **Reference paths.** Folders marked as reference are compared against but never modified or deleted. Supported by duplicates, similar images, videos and music.
- **Exclusions.** Excluded paths plus excluded-item globs (for example `*/.git`) matched against the full path.
- **Empty folders.** Start with every directory as a candidate, un-mark any directory that holds a file or a non-empty subdirectory, propagate upward, and report what remains.
- **Temporary file patterns.** `.tmp`, `.bak`, `.part`, `.crdownload`, `.temp`, `.cache`, `.dmp`, `.download`, `.partial`, `thumbs.db`, `~`, `#`. A custom list replaces the defaults. Note that `.bak` and `.dmp` can be user-valuable, so on Windows treat these as "review", not "safe".
- **Similar images.** Perceptual hash (Gradient, Mean, Blockhash, others), hash size 8 to 64, a BK-tree, and a Hamming-distance threshold. Mirror and rotate invariance is optional.
- **Broken files.** Detected by actually parsing the file. Documented as prone to false positives, so treat results as "review".
- **Delete methods.** Delete, all-except-newest/oldest/biggest/smallest, one-newest and similar, hard link, move to trash and dry run.
- **Krokiet UX.**
  - Grouped result rows and a preview pane.
  - Select helpers, including "except" variants and invert within a group.
  - Context menu: open parent, exclude item.
  - Presets saved as JSON.
  - The language can be switched without a restart.
- **Crates in `czkawka_core`.** `blake3`, `xxhash-rust` (xxh3), `crc32fast`, `rayon`, `trash`, `image_hasher`, `bk-tree`, `bincode`, and `file-id` on Windows (used for hard-link and file identity).
- **Trust statement.** The README says the app has no internet access and collects no data. We should make the same claim and keep it true.

### 3.4 GUI space analyzers

- **WinDirStat 2.x (verified).**
  - Visualization: treemap, sunburst and flame graph, largest files, and file type stats.
  - Scanning: accelerated local NTFS scanning when elevated, pause/resume, and multiple targets.
  - Duplicates: off by default because hashing reads files. Configurable hashes, "probable duplicates" from sparse hashing, and cloud-file safeguards.
  - Size and links: shows logical vs physical size and hard links.
  - Cleanup: NTFS compression, hardlink dedup, custom cleanup commands, and Windows cleanup tools.
  - Data and display: CSV/JSON save and load, a filesystem monitor, and dark mode.
- **WizTree (verified).** Reads the MFT directly on NTFS, counts hard-linked files once, and claims allocated sizes match Windows. Free for personal use only. Whether MFT reading needs admin rights is not stated on the page; reading a raw volume does require elevation in general (unverified).
- **SpaceSniffer (secondary).** A treemap-first portable freeware tool. Filters combine a mask with size and age (for example images over 1 MB younger than 3 months). Files can be tagged by color, and a pipe prefix excludes matches. May miss protected files when not elevated.
- **QDirStat (verified).**
  - Unlimited user-defined cleanup actions (shell commands) with an output window and a kill button.
  - MIME/extension categories with treemap colors.
  - File type stats with "where are these files" drill-down, and file age stats by year and by recent months.
  - Exclude rules match the last path component by default, with regex, wildcard or fixed-string modes.
  - A gzip scan cache. Does not cross filesystem boundaries by default.
- **TreeSize Free (secondary).** Uses the MFT on local NTFS and falls back to two threads elsewhere. Duplicates, top-100 files and bulk operations are paid-tier features.
- **Treemap algorithm.** The squarified layout (Bruls, Huizing and van Wijk, 2000) keeps rectangles close to square so they are easier to compare and click. Cushion shading (an earlier paper by the same group) adds depth cues for hierarchy. Whether WinDirStat uses both was not confirmed in its README (unverified, widely reported).

### 3.5 CLI analyzers and walkers (verified unless noted)

- **dust.**
  - Uses `rayon`. On Windows it also uses `winapi-util` and `filesize` (the latter returns on-disk size).
  - The walker thread count defaults to the CPU count. More threads help on high-latency storage.
  - Hard links are counted once by default.
  - Offers type grouping (`-t`), JSON output, and a screen-reader mode (`-R`).
- **dua-cli.**
  - "Parallel by default." Its walker now lives in its own `dua-core` crate, which uses `crossbeam` and `windows-sys`.
  - Interactive mode marks entries, then deletes or trashes them in a multi-stage process.
  - The delete and trash actions can each be disabled in config.
  - `dua clean` lists disposable directories and deletes nothing automatically. It skips directories containing `.git`; inside a repo, candidates must be ignored and must contain no tracked files.
  - Does not follow symlinks.
  - Memory cost is about 60 MB per million entries.
  - UI is localized in 5 languages.
- **gdu.**
  - Parallel scan aimed at SSDs, with `--sequential` for HDDs.
  - Hard links are counted once and flagged `H`.
  - Export/import JSON, with SQLite or Badger storage.
  - The web UI binds to localhost and needs a per-run token to delete.
  - `--no-delete` disables deletion, and deleting while filters are active is blocked by default.
  - Supports mtime filters (`--since`/`--until`).
- **ncdu 2.x.**
  - Multithreaded scanning is opt-in (`-t8`).
  - The binary export format supports trees larger than memory.
  - Colors are off by default for legibility.
- **diskus.** About 10x faster than `du -sh` on a cold cache in its own benchmark. On Windows it does **not** dedupe hard links or junctions, which matches Explorer. Useful as a reminder that "match Explorer" and "be accurate" are different goals, and we should show both.
- **pdu.** Hardlink dedup is optional because it makes scans slower. JSON in and out.
- **jwalk.** Parallel, sorted, streaming walk on rayon with a per-directory callback. **Archived and unmaintained.** Alternatives: `ignore::WalkBuilder::build_parallel` (the ripgrep walker, unverified that it fits our needs), or a custom rayon/crossbeam walker over `std::fs::read_dir`, which on Windows yields metadata without an extra system call.

### 3.6 Developer artifact cleaners (verified)

- **kondo marker table** (from `kondo-lib/src/lib.rs`):

  | Project | Marker | Artifacts |
  |---|---|---|
  | Cargo | `Cargo.toml` | `target`, `.xwin-cache` |
  | Node | `package.json` | `node_modules`, `.angular` |
  | React Native | `package.json` plus `ios` or `android` | `android/build`, `android/.gradle`, `ios/build`, `ios/Pods`, `.expo`, `.metro` |
  | Unity | `Assembly-CSharp.csproj` | `Library`, `Temp`, `Obj`, `Logs`, `MemoryCaptures`, `Build`, `Builds` |
  | Stack | `stack.yaml` | `.stack-work` |
  | Cabal | `cabal.project` | `dist-newstyle` |
  | SBT | `build.sbt` | `target`, `project/target` |
  | Maven | `pom.xml` | `target` |
  | Gradle | `build.gradle`, `build.gradle.kts` | `build`, `.gradle` |
  | CMake | `CMakeLists.txt` | `build`, `cmake-build-debug`, `cmake-build-release` |
  | Unreal | `*.uproject` | `Binaries`, `Build`, `Saved`, `DerivedDataCache`, `Intermediate` |
  | Jupyter | `*.ipynb` | `.ipynb_checkpoints` |
  | Python | `*.py` | `__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.tox`, `.nox`, `__pypackages__` |
  | Pixi | `pixi.toml` | `.pixi` |
  | Composer | `composer.json` | `vendor` |
  | Dart/Flutter | `pubspec.yaml` | `build`, `.dart_tool`, platform `flutter/ephemeral` |
  | Elixir | `mix.exs` | `_build`, `.elixir_ls`, `.elixir-tools`, `.lexical` |
  | Swift | `Package.swift` | `.build`, `.swiftpm` |
  | Zig | `build.zig` | `zig-cache`, `.zig-cache`, `zig-out` |
  | Godot 4 | `project.godot` | `.godot` |
  | .NET | `*.csproj`, `*.fsproj` | `bin`, `obj` |
  | Turborepo | `turbo.json` | `.turbo` |
  | Terraform | `.terraform.lock.hcl` | `.terraform` |
  | CocoaPods | `Podfile` | `Pods` |

  kondo stops descending once it matches a project. Its "last modified" walks the whole project, including artifact dirs. A better approach for us is to exclude artifact dirs when computing project activity, so a recent build does not make an abandoned project look active (our own design choice).
- **npkill profiles.** The default node profile covers `node_modules`, `.next`, `.nuxt`, `.svelte-kit`, `.vite`, `.turbo`, `.parcel-cache`, `.angular`, `coverage`, `storybook-static`, `.docusaurus`, `.swc` and more. Other profiles: python, java, android, swift, dotnet, rust, ruby, elixir, haskell, scala, cpp, unity, unreal, godot, infra.
  - The docs flag risky names that need confirmation: `.venv`/`venv` (removes environments), `coverage`/`htmlcov`/`TestResults` (history lost), `.vs` (user IDE settings), `.dvc`/`.mlruns` (possible data loss), and generic names that collide with unrelated folders (`out`, `deps`, `target`, `Obj`, `Intermediate`, `Library`, `Temp`).
  - npkill also flags `node_modules` belonging to installed applications, because deleting them breaks the app.
- **cargo-sweep.** Removes artifacts older than N days, artifacts not built by the installed toolchains, or artifacts until `target` is under a size budget. Supports a stamp-file workflow and a dry run. Unmaintained.
- **Mole purge.**
  - Protects projects with deployment keypairs, nested git repos or git-tracked files.
  - Leaves recently active artifacts unselected.
  - Requires `--yes` for non-interactive runs.

### 3.7 Mole and similar polished cleaners (verified unless noted)

- **Commands.**
  - `clean`: caches, logs, temp, dev artifacts, leftovers.
  - `uninstall`: app plus launch agents, prefs and leftovers.
  - `optimize`: bounded maintenance that skips unsafe or unnecessary tasks.
  - `analyze`: a disk explorer that moves items to Trash after confirmation.
  - `status`: a read-only health dashboard.
  - `purge`: dev artifacts.
  - `installer`: finds DMG, PKG, ISO and installer ZIP files.
  - `history`: the operations log.
- **Safety design.**
  - `--dry-run` on every destructive command.
  - Path validation; shared or system locations are refused when safety cannot be proven.
  - Runs without admin and asks only when needed.
  - Re-checks a file's identity right before deletion and does not follow symlinks under scan roots.
  - Lists references in shell and config files but does not edit them.
  - Logs every operation to a local file, which `history` shows; logging can be disabled.
  - Allowlists for clean and optimize. **If the allowlist cannot be read, the command stops** (fail closed).
  - JSON output for automation.
- **Note on the clean default.** `mo clean` empties the Trash by default. That is the opposite of what we want (see section 6).
- **Mole for Windows.** A Windows CLI exists on a separate branch, and native paid apps are sold separately (verified that they exist; not reviewed).
- **Pearcleaner (macOS).** Uninstall with leftovers, orphaned file search, a privileged helper used only for system folders. On hold.
- **Microsoft PC Manager (secondary).** Deep cleanup per app cache, large files, duplicate files, a Downloads overview and startup apps. A useful reference for what Windows users already expect.
- **Newer Rust/Tauri Windows cleaners (secondary, small).** WindowsClear (about 740 stars), AppDataCleaner (egui, about 260 stars), NeatDisk (Tauri, launched May 2026), Skiff, Safai. None is dominant, so there is room for a polished, safe, open-source Windows option.

### 3.8 Bulk Crap Uninstaller (verified for the official site; confidence details secondary)

- **Detection.**
  - Registered apps, including hidden or protected entries.
  - Apps with damaged or missing uninstallers.
  - Portable apps in common locations.
  - Chocolatey, Steam, Oculus, Windows Store apps, Windows Features and Windows Updates.
- **Uninstall.**
  - Runs the original uninstaller first.
  - Can run quietly even when the uninstaller has no silent mode.
  - Batch removal with collision prevention.
  - Handles hung uninstallers.
  - Force uninstall as a last resort.
  - Verifies uninstaller certificates.
- **Leftover scan (secondary, from a third-party wiki; verify in the app).**
  - Scans registry (HKLM and HKCU), install folders, AppData and ProgramData, services, scheduled tasks, Run keys, startup folders and shortcuts.
  - Four confidence levels: Very good, Good, Questionable, Bad.
  - Scoring factors: location, name similarity, publisher, dates and signatures.
  - Only Very good and Good are preselected; Questionable and Bad are hidden by default.
  - Removal order: commands, startup items, registry keys, files, shortcuts.
  - Optional backup as `.reg` exports plus an archive of removed files.
  - Changelogs mention a check for similarly named apps to cut false positives (secondary).
- **Other tools.** Program Files orphan cleanup, startup manager, uninstall lists with regex filters.
- **Status.** The project is looking for maintainers.

### 3.9 Tweakers: Optimizer, Win11Debloat, Sophia Script (verified)

- **Optimizer.** Archived January 2026 and replaced by OptimizerNXT. It mixes junk cleaning with service disabling, stopping Windows Update, disabling Defender (a command-line switch) and a "registry fixer". **Avoid all of that in a cleaner.**
- **Win11Debloat.** App removal, telemetry, AI and UI tweaks, and Storage Sense toggles. Needs admin. Claims changes are easily revertible. **Out of scope.** One pattern worth borrowing: exporting and importing the selected settings.
- **Sophia Script.**
  - Each tweak is a function with an explicit argument and a matching default-restoring call; a commented preset file selects what runs.
  - Cleanup-related scheduled tasks: Windows cleanup, `SoftwareDistribution\Download`, and temp folders such as `$WinREAgent`, `$SysReset`, `CrashDumps`.
  - The function-per-change design is a good model for our PowerShell export.

### 3.10 Windows built-ins (verified unless noted)

- **cleanmgr and VolumeCaches.**
  - Handlers live under `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VolumeCaches\<name>`.
  - The data-driven handler (CLSID `{C0E13E61-0CC6-11d1-BBB6-0060978B2AE6}`) is purely declarative. Its values:
    - `Folder` (with `?:` for the current drive), optionally prefixed by a `CSIDL` value.
    - `FileList` (wildcards separated by `|`).
    - `LastAccess` (minimum days).
    - `Flags`: recurse into subdirs, remove read-only/system/hidden files, don't show if zero, match directories, run only when out of space, remove the parent dir.
    - `Priority`.
    - `Display`, `Description`, `PropertyBag` (localization), `IconPath`.
  - `cleanmgr /sageset:n` stores per-handler selections as `StateFlagsNNNN` (0 = skip, 2 = run), and `/sagerun:n` runs them. A scheduled run sets a mode in which handlers should only touch extremely safe files.
  - **Ideas to borrow:** read the installed handlers to show "Windows-registered cleanup categories", and model our rule fields on these values (min age, flags, priority, localized display).
  - **Do not write** `StateFlags` keys or register handlers. That is persistent configuration of the OS.
- **Downloads in Disk Cleanup (secondary).**
  - Windows 10 1809 added a "Downloads" option.
  - Users who habitually ticked every box lost their Downloads folders.
  - Microsoft removed the option in 20H1 based on feedback.
  - **Lesson:** never put user-content folders in a "select all" cleaner list.
- **Storage Sense.**
  - Off by default, but Windows may turn it on when space is low.
  - Cleans temp files, the Recycle Bin after N days, and Downloads only if the user sets an age.
  - Makes inactive OneDrive files online-only (default 30 days on 22H2+); nothing is deleted.
  - System drive only. Runs only after the user has been signed in and online for more than 10 minutes.
- **Cleanup recommendations (secondary).** Settings > System > Storage groups suggestions into temp files, large or unused files, files synced to the cloud (local copies can be freed), and unused apps. Nothing is removed without confirmation. **This four-bucket layout is a good template for our overview dashboard.**
- **Component store (WinSxS).**
  - Never delete files from `WinSxS`.
  - Explorer overstates its size because of hard links. `DISM /Online /Cleanup-Image /AnalyzeComponentStore` reports the actual size, the part shared with Windows, backups, cache, the number of reclaimable packages and whether cleanup is recommended.
  - The `StartComponentCleanup` scheduled task waits 30 days and has a 1-hour timeout. `DISM /StartComponentCleanup` removes superseded components immediately.
  - Adding `/ResetBase` makes **existing updates impossible to uninstall**, so it is expert-only.
- **Windows.old.**
  - Removed automatically about 10 days after a feature update.
  - Deleting it removes the ability to go back.
  - Microsoft's recommended route is Settings > System > Storage > Temporary files > Previous Windows installation(s).
  - Some sources say the rollback window can be extended up to 60 days (secondary).
- **Windows ESD installation files (secondary).** These back "Reset this PC". Deleting them breaks local reset unless the user resets via cloud download. Mark as dangerous.
- **`C:\Windows\Installer` (secondary).** Holds cached MSI/MSP files needed for repair, patch and uninstall. Wiping it breaks those operations, sometimes much later. Only orphan-aware tools (PatchCleaner-style) should touch it. **Out of scope for us.**
- **Hibernation.** `powercfg /hibernate off` (admin) removes `hiberfil.sys` (about RAM-sized). It also disables hybrid sleep, and Microsoft warns of data loss on power failure with hybrid sleep. Fast Startup also depends on hibernation (unverified). Present this as a setting with its consequences, not as "junk".
- **Delivery Optimization.** `Delete-DeliveryOptimizationCache` is in the DeliveryOptimization PowerShell module. Parameters: `-Force`, `-IncludePinnedFiles`, `-FileId`. The Microsoft page is a stub; admin requirement and behavior are from secondary sources. Also exposed in Settings > Storage > Temporary files and in cleanmgr.
- **System Restore.** `Checkpoint-Computer` creates a restore point (client OS only, Windows PowerShell 5.1) and is limited to one per 24 hours.
- **Restart Manager.** The API that installers use to find and close apps holding files. We can use `RmStartSession`, `RmRegisterResources` and `RmGetList` read-only, to show which process locks a file (function names from the reference; usage unverified in our context).

### 3.11 Commercial references (feature ideas only)

- **CCleaner.**
  - Per-app cleaning lists, scheduled cleaning, startup manager, uninstaller and duplicate finder.
  - Trust incidents:
    - 2017: a signed, trojanized build (v5.33) reached about 2.3M users.
    - 2018: v5.45 "Active Monitoring" could not be turned off and the privacy options were removed. The version was pulled and 5.46 restored the opt-out.
    - 2020: Microsoft Defender flagged some free installers as PUA because they bundled other products.
  - CCleaner also shipped a registry cleaner.
- **Wise Disk Cleaner (secondary).**
  - "Advanced cleaning": pick drives and file types, see results with last-access times.
  - Scheduled auto-clean daily, weekly, monthly or at logon.
  - "System slimming" removes Windows components (sample media, IMEs, update installers). **We will not ship slimming.**
- **PrivaZer (secondary).**
  - Deep "traces" cleaning, including MFT records, free space, the USN journal and unallocated registry space, plus a registry backup.
  - Reviewers recommend unchecking its registry cleaner.
  - Slow, and all-or-nothing within a location.
- **TreeSize.** The paid tier adds duplicates, top-100 files, scheduled scans and bulk operations. Free has the MFT scan and treemap.

---

## 4. Implementation notes for Rust and Windows

These are concrete techniques collected from the projects above. Items marked (unverified) come from general Windows knowledge and need a test before we rely on them.

### Scanning

- **Walk first, MFT later.**
  - Start with a parallel walk over `std::fs::read_dir`. On Windows, `DirEntry::metadata()` and `file_type()` are cheap because `FindNextFileW` already returns the data (verified, Rust docs).
  - Distribute directories over a rayon or crossbeam work queue (dust uses rayon; dua-core uses crossbeam).
  - Do not depend on `jwalk` (archived).
- **Optional fast path (P2).** Read the MFT when elevated, as WizTree, TreeSize and WinDirStat 2.x do. Candidate crates: `ntfs-reader` (reads MFT and USN), `usn-journal-rs`, `ntfs` (Colin Finck). All need elevation (secondary). Always keep the walker as the fallback for non-NTFS, network and unelevated runs.
- **Logical vs on-disk size.** `metadata.len()` is the logical size. On-disk size needs `GetCompressedFileSizeW` (dust's `filesize` crate does this on Windows) plus cluster rounding (unverified detail). Show both, as WinDirStat does.
- **Hard links.**
  - Count each file identity once (volume serial plus file index). Czkawka uses the `file-id` crate on Windows; `same-file` is another option (unverified fit).
  - Getting the file index needs a handle per file, which is costly. Dedupe only for files whose link count is above 1, which also needs a handle (unverified). Alternatively, dedupe only in "accurate mode", as pdu makes hardlink dedup optional.
- **Reparse points.** Never follow junctions, symlinks or mount points while scanning or deleting.
  - Check `FILE_ATTRIBUTE_REPARSE_POINT` on every entry (unverified constant use).
  - The user profile contains legacy junctions such as `Application Data` and `Local Settings`; following them loops or double-counts (unverified, widely known).
  - BleachBit 6.0 fixed a data-loss bug caused by following junctions when emptying the Recycle Bin.
- **Cloud placeholders (OneDrive and similar).**
  - Check `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (0x00400000) and `FILE_ATTRIBUTE_RECALL_ON_OPEN` (0x00040000) before any content read (secondary).
  - Never hash these files and never count them as local usage. Report them as cloud-only. WinDirStat mentions "cloud-file safeguards" for the same reason.
- **HDD mode.** Offer a sequential mode for spinning disks, as gdu does.
- **Partial results.** Mark partial results clearly when access is denied or a scan is cancelled (Mole marks partial scans; QDirStat reports what it skipped).
- **Snapshots.** Save scan snapshots (compressed JSON or binary) so the treemap opens instantly and two scans can be compared over time (ncdu, gdu, QDirStat, pdu).

### Deleting

- **Use the Recycle Bin.** Recycle via `IFileOperation`, which Microsoft recommends over `SHFileOperation` since Vista, with undo flags. The `trash` crate (used by dua and Czkawka) wraps this.
  - **Always pass fully qualified paths.** Microsoft notes that `SHFileOperation` will not recycle a file given without a full path, and that it fails on `\\?\` paths.
  - Detect when recycling would become a permanent delete and ask first. This happens when an item is larger than the bin quota (secondary) and on volumes without a Recycle Bin, such as many removable and network drives (unverified). `FOF_WANTNUKEWARNING` exists for exactly this case (verified).
- **Re-validate just before delete (Mole).** Check that the path still exists, is still under the expected root, is not a reparse point, and has the same size and modification time as in the preview. Skip anything that changed.
- **Locked files.** Skip them and report them. Use Restart Manager to name the locking process. Do not kill processes.
- **Treat directories with care.** Delete a directory only when the rule says the directory itself is the target (BleachBit's `walk.top` vs `walk.all` distinction). Otherwise delete its contents and keep the folder, because some apps expect their cache folder to exist.

### Uninstall registry (verified fields; usage notes unverified)

- **Where to read.**
  - `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`
  - The same path under `WOW6432Node` for 32-bit apps
  - `HKCU\...\Uninstall` for per-user installs
- **Fields documented by Microsoft.** `DisplayName`, `DisplayVersion`, `Publisher`, `InstallDate`, `InstallLocation`, `InstallSource`, `EstimatedSize`, `UninstallString`, `ModifyPath`, URLs.
- **Widely used but not on that page (unverified).**
  - `QuietUninstallString`.
  - `SystemComponent=1` (hidden from Apps and Features).
  - `NoRemove` and `NoModify`.
  - `ParentKeyName` and `ReleaseType` (updates and patches).
  - `WindowsInstaller=1` (MSI product code is the key name).
  - `EstimatedSize` is in KB.
- **Uninstall flow.** Run the vendor uninstaller and wait for it. Re-scan the Uninstall keys to confirm removal. Only then offer a leftover scan, following BCU's order and confidence model.

### Startup manager (secondary)

- **Sources to list.**
  - `HKCU` and `HKLM` `...\CurrentVersion\Run` and `RunOnce`, including `WOW6432Node`.
  - The user and common Startup folders.
  - Read-only: scheduled tasks with logon triggers, and services.
- **How to disable.** Disable the way Task Manager does, through `...\Explorer\StartupApproved\Run` (and `StartupFolder`). A value whose first byte is `02` means enabled, `03` means disabled (byte layout from secondary sources; verify on a live system).
  - This is reversible and shows up in Task Manager.
  - Never delete Run values by default.
  - Autoruns uses its own separate mechanism.

### PowerShell export

- **Model.** Follow the Sophia function-per-change idea. Every item is a commented line with its size and risk, and the script supports `-WhatIf` style dry runs.
- **Safe deletion syntax.**
  - Use `-LiteralPath` so that `[` and `]` in names are not treated as wildcards (unverified but standard PowerShell behavior).
  - Recycle via `[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile/DeleteDirectory(..., 'OnlyErrorDialogs', 'SendToRecycleBin')` (unverified API usage; test on PowerShell 5.1).
- **Admin steps.** Put admin-only steps (DISM, `powercfg`) in a separate section guarded by an elevation check.
- **Syntax target.** Windows PowerShell 5.1: no `&&` or `||`, and no ternary operator.

---

## 5. Common user complaints about cleaners

| Complaint | Examples | Our answer |
|---|---|---|
| Deleted something they needed | Disk Cleanup "Downloads" option wiped folders (1809); deleting `C:\Windows\Installer` breaks MSI uninstall/repair; deleting ESD files breaks Reset; deleting node_modules of installed tools broke editors; Recycle Bin junction-following bug in BleachBit | Preview, Recycle Bin, protected-path deny list, no user-content folders in cleaner rules, risk levels, never follow reparse points |
| Cleaned but nothing changed / numbers wrong | BleachBit reports of browser data still present; repeated "1.8 GB freed"; Explorer overstating WinSxS; double-counted hard links | Re-scan after cleanup and report the actual freed space (volume free space before vs after), show logical vs on-disk size, dedupe hard links |
| Bundled adware, upsell, nag screens | CCleaner installers bundling other products, flagged as PUA by Defender in 2020 | Single MIT installer, no offers, no paid tier inside the app, no popups |
| Telemetry and always-on monitoring | CCleaner 5.45 Active Monitoring could not be disabled | No telemetry, no background service, no network calls except an opt-in update check that only notifies (BleachBit model) |
| Supply-chain trust | CCleaner 5.33 signed malware build (2017) | Reproducible CI builds, signed releases with published checksums, minimal dependencies, `cargo audit` and `cargo deny` in CI |
| Registry cleaner broke Windows | Microsoft KB 2563254: Microsoft does not support registry cleaners, and problems may require reinstalling Windows | No registry cleaner (section 7) |
| Slow scans | Deep scans in BleachBit and PrivaZer; Krokiet duplicate scans on big drives | Parallel walk, cached hashes, staged duplicate pipeline, snapshots, cancel at any point |
| Antivirus false positives | Cleaners and tweakers often trip heuristics (common report, unverified) | Code signing, no self-elevation tricks, no process killing, no driver installs |
| Too many confirmations or none at all | BleachBit's per-file confirmations in old shredder; one-click "clean all" in others | One summary confirmation per batch, stronger confirmation only for "dangerous" items, remember-choice only in expert mode |

---

## 6. Safety principles

1. **Preview is the default path.** Every destructive action first produces a plan: the exact items, sizes, the rule that matched, a risk level and the delete method. The user executes that plan, not a new scan. Commands that run without a UI need an explicit flag (`--yes`, as in Mole and npkill).
2. **Recycle Bin by default.** Permanent deletion is a separate, explicitly labeled action. When the bin cannot take an item (too big, or no bin on that volume), say so and ask. Never empty the Recycle Bin implicitly; Mole does, and we deliberately differ.
3. **Hard deny list (protected paths), not overridable by rules.**
   - Windows system folders: `%WINDIR%` except named temp/log targets, `WinSxS`, `System32`, `Installer`, `SoftwareDistribution` except `Download` via a rule.
   - Program Files roots, `ProgramData` roots, and drive roots.
   - The user's Documents, Desktop, Pictures, Music, Videos and Downloads (only the large-file and duplicate finders may show these, with Review risk).
   - OneDrive and other sync roots.
   - `.git` directories.
   - The app's own folders.
4. **Rules target explicit locations** (Winapp2 philosophy). No broad "every *.bak on C:" style rules in the safe tier.
5. **Allowlist and exclusions everywhere.** User exclusions (paths and globs, Czkawka style) apply to scan, preview and delete. If the allowlist or config cannot be read, destructive commands stop (Mole fails closed).
6. **Three risk levels.**
   - **Safe:** regenerable caches. Preselected.
   - **Review:** may contain user value, or was recently used. Not preselected.
   - **Dangerous:** hidden behind expert mode, with a per-item warning (BleachBit 5.1+ model).
   - Also use a minimum age (`LastAccess` in VolumeCaches, `--older` in kondo), so recently touched items drop to Review.
7. **Process and lock checks.** Skip an app's cache while the app runs (BleachBit `running`). Report locked files with the locking process (Restart Manager). Never kill processes automatically.
8. **Least privilege.** Run as a normal user. Elevate only for named system actions (DISM, hibernation, all-users temp, HKLM startup entries), and show the exact command first. Implement elevation as a separate elevated helper process with a narrow command set, never as "restart the whole app as admin" (Microsoft guidance; Mole, Pearcleaner).
9. **No following of links and no TOCTOU.** Do not traverse reparse points. Re-validate identity and location just before deleting (Mole).
10. **Leave cloud and remote data alone.** Never hydrate cloud placeholders, never hash them, and do not scan network drives unless the user explicitly adds them.
11. **History and honesty.** Append every operation to a local history (time, rule, path, size, method, result, freed bytes measured by volume free space before and after). Provide "Restore from Recycle Bin" where possible. Never claim undo for permanent deletes.
12. **System areas only through official tools.** Use DISM, `Delete-DeliveryOptimizationCache`, `powercfg`, and the Settings or cleanmgr route for Windows.old. Never delete files inside servicing stores directly.
13. **Restore point before system-level changes** (uninstall, startup changes in HKLM) when System Restore is enabled. Respect the one-per-24-hours limit and say when one was skipped.
14. **Uninstall: vendor uninstaller first, leftovers second, confidence-gated** (BCU).
15. **Same safety in the PowerShell export.** The generated script carries the same checks: protected paths, `-LiteralPath`, recycle-bin deletion, and a dry-run switch on by default.

---

## 7. Things we deliberately will not do

- **No registry cleaner.** Microsoft's KB 2563254 says Microsoft does not support registry cleaners and that problems they cause may require reinstalling Windows. Reviewers advise unchecking PrivaZer's registry cleaner, and the gains are negligible. Registry access is limited to reading (uninstall entries, startup entries, VolumeCaches) plus StartupApproved toggles.
- **No "system slimming" or component removal.** Wise-style removal of Windows components, IMEs, sample media, ESD files or `Installer` cache contents.
- **No debloating or privacy tweaks.** No telemetry toggles, no Copilot or Recall changes, no Store app removal, no service disabling, no hosts editing, no Defender or Windows Update changes (Optimizer, Win11Debloat and Sophia cover that space).
- **No "PC booster" or RAM cleaner,** and no prefetch, superfetch or font-cache "optimizations" sold as speedups.
- **No shredding or free-space wiping marketed as secure.** BleachBit's own docs say it is best effort on SSDs. If ever added, label it accordingly and put it behind expert mode.
- **No `DISM /ResetBase` by default** (it blocks uninstalling existing updates). Expert mode only, with a warning.
- **No manual deletion** of `WinSxS`, `C:\Windows\Installer`, `Windows.old` contents (use the official route) or `hiberfil.sys` (use `powercfg`).
- **No user-content folders in cleaner rules.** No Downloads cleaning, no Documents. Large-file and duplicate finders may show them, but only as Review and never preselected.
- **No automatic or scheduled deletion.** A scheduled scan with a notification is acceptable later; a scheduled delete is not.
- **No telemetry,** no analytics SDKs, no crash upload without explicit consent, and no background service or tray monitor.
- **No bundling, ads, upsells, nag screens, or fake "issues found" counters.**
- **No killing of processes** to free locks, and no delete-on-reboot by default.
- **No copying of GPL code or CC-BY-SA rule files** (BleachBit cleaners, Winapp2.ini, Mole, WinDirStat, QDirStat, Krokiet) into this MIT project. Ideas yes, code and data no.

---

## 8. Accessibility and i18n patterns

- **Treemap is not the only view.** A treemap is inaccessible to screen readers and to many keyboard users. Pair it with a synchronized tree table (WinDirStat, QDirStat, K4DirStat). The tree table should use the WAI-ARIA treegrid pattern with full keyboard navigation: arrows, Enter to drill in, Backspace to go up, Space to select.
- **Do not encode meaning in color alone.** Extension colors in the treemap need a legend and hover or focus text. Risk levels need text labels and icons, not only red, orange and green.
- **Screen-reader mode idea.** dust has a screen-reader output mode (`-R`). In the UI, use live regions for scan progress, kept low-frequency.
- **Reduced motion.** Respect `prefers-reduced-motion` for treemap zoom animations.
- **Keyboard for everything,** with visible focus. Bulk actions need keyboard shortcuts and a confirmation that can be operated by keyboard.
- **Language switching.** Switch immediately without a restart (Krokiet). Store labels with ICU MessageFormat for Turkish and English plurals.
- **Rule localization.** Rule labels and descriptions are localized fields inside the rule file (BleachBit exports to gettext; VolumeCaches uses `PropertyBag`).
- **Turkish casing pitfall (unverified detail, well known).** Never use locale-aware lowercasing (`toLocaleLowerCase('tr')`, or ICU case mapping with a Turkish locale) for path or extension matching, because `I` maps to dotless `ı`. Use ordinal, locale-invariant, case-insensitive comparison, as NTFS does (Rust's `str::to_lowercase` is locale-independent).
- **Number and size formatting.** Format sizes per UI locale (Turkish uses a comma as the decimal separator). Pick one unit convention and say which: Windows Explorer uses 1024-based units labeled KB/MB/GB. Use the same labels as Explorer so numbers match what users see, and note it in the docs.
- **Dates.** Show relative ages ("3 months ago") plus an absolute date on hover. QDirStat offers an ISO format option.

---

## 9. Feature ideas, prioritized

P0 = needed for the first public release. P1 = next. P2 = later or optional.

### P0

| # | Feature | Inspired by |
|---|---|---|
| P0-1 | Parallel scanner (rayon/crossbeam over `read_dir`), cancel/pause, partial-result marking, logical and on-disk size, hard links counted once, no reparse-point following | dust, dua, gdu, pdu, WinDirStat, Mole |
| P0-2 | Overview dashboard with buckets "Safe to remove", "Needs review", "Free after cleanup" (projected), plus volume free-space bar | Windows Cleanup recommendations, Mole status |
| P0-3 | Declarative rule engine (TOML or JSON): `id`, localized `label`/`description`, `category`, `risk`, `os` and min-Windows-build guard, `running` process guard, base `paths` with env/known-folder variables, `search` mode (file, glob, walk.files, walk.all, walk.top), include/exclude globs and regex, `type` filter, `min_age_days`, `delete_dir_itself` flag, `requires_admin`, `warning` text, `regenerates` note. Schema-validated at load | BleachBit CleanerML, VolumeCaches, Winapp2 |
| P0-4 | Initial safe rule set: user and system Temp (age-gated), Windows Error Reporting queues, thumbnail and icon caches, browser caches (cache only, never cookies or passwords) with running guards, `SoftwareDistribution\Download` (admin), Delivery Optimization (cmdlet), crash dumps (Review), log files (Review) | cleanmgr handlers, Sophia Script, BleachBit |
| P0-5 | Preview-then-execute plan for every destructive action, with per-item rule, size and risk | BleachBit, Mole `--dry-run`, npkill, cargo-sweep |
| P0-6 | Recycle Bin by default via `IFileOperation`/`trash` crate, full paths only, a warning when an item would be permanently deleted, permanent delete as a separate action | dua, gdu, Krokiet, Microsoft docs |
| P0-7 | Protected-path deny list plus user allowlist (paths and globs), fail closed when config is unreadable | Mole, BleachBit allowlist, Czkawka excluded items |
| P0-8 | Running-process and locked-file checks (skip and report, never kill) | BleachBit `running`, BCU |
| P0-9 | Treemap (squarified, cushion-style shading optional) synced with an accessible tree table, drill-down, breadcrumbs, extension/type stats panel with "show where" | WinDirStat, QDirStat, SpaceSniffer, WizTree |
| P0-10 | Large file finder: top N with size, age and type filters, open location, recycle | Czkawka big files, WinDirStat, PC Manager |
| P0-11 | Developer artifact cleaner: marker-file table (kondo list plus `.next`, `.nuxt`, `.svelte-kit`, `.turbo`, `.parcel-cache`, `coverage` from npkill), activity age excluding artifact dirs, recent projects unselected, skip git-tracked content, skip `node_modules` inside installed apps (Program Files, AppData app dirs), risky names (`.venv`, `.vs`, `out`, `deps`) as Review | kondo, npkill, Mole purge, dua clean, cargo-sweep |
| P0-12 | Duplicate finder: size grouping, then first/last 4 KB prehash, then full BLAKE3 (xxh3 option); hash cache keyed by path, size and mtime; skip cloud placeholders and zero-byte files; reference folders that are never selected; group selection helpers (keep newest, oldest, or the shortest path) | Czkawka/Krokiet, WinDirStat |
| P0-13 | Empty folder finder with bottom-up propagation; exclude protected paths and folders holding only `desktop.ini` as Review | Czkawka |
| P0-14 | Installed apps list from HKLM, WOW6432Node and HKCU Uninstall keys, hide `SystemComponent` and update entries, show size, publisher, install date, location; run the vendor uninstaller and verify removal | BCU, Microsoft Uninstall key docs |
| P0-15 | Startup manager: list Run/RunOnce and Startup folders (scheduled tasks and services read-only), enable/disable via StartupApproved (reversible), open location, no deletion by default | Task Manager, BCU, Optimizer |
| P0-16 | Cleanup history: local append-only log (JSON Lines) with rule, path, size, method, result, measured freed bytes; history screen with filter and "Open Recycle Bin" | Mole operations log and `history`, BCU backups |
| P0-17 | "Generate PowerShell script" from any preview plan: commented, `-LiteralPath`, recycle-bin deletion, dry-run switch default on, admin section separated | Sophia Script, Mole dry run |
| P0-18 | Elevation on demand through a separate helper for named system actions only, with the exact command shown first | Mole, Pearcleaner, Microsoft least-privilege guidance |
| P0-19 | Turkish and English UI with instant switching, ICU plurals, locale-invariant path matching | Krokiet, dua localization |
| P0-20 | No telemetry, no network by default; opt-in update check that only notifies | Czkawka statement, BleachBit update check |

### P1

| # | Feature | Inspired by |
|---|---|---|
| P1-1 | Leftover scan after uninstall with four confidence levels, only the top two preselected, ordered removal, optional `.reg` export and archive backup of removed files | BCU |
| P1-2 | Quiet and batch uninstall (`QuietUninstallString`, MSI quiet mode) with one confirmation and per-app progress | BCU |
| P1-3 | Expert mode: dangerous rules locked, per-rule warning dialog, "remember" only in expert mode, reset of remembered choices | BleachBit 5.1/6.0 |
| P1-4 | Windows system tools panel: component store analysis plus `StartComponentCleanup` (no ResetBase), Delivery Optimization cleanup, previous Windows installation via the official route, hibernation toggle with consequences explained | Microsoft Learn, Sophia Script |
| P1-5 | Scan snapshots: save, reopen instantly, compare two snapshots ("what grew since last week") | ncdu, gdu, QDirStat, pdu |
| P1-6 | File age statistics (by year and recent months) for any folder | QDirStat |
| P1-7 | Compact filter syntax in the explorer (mask; size; age) with chips | SpaceSniffer, gdu `--since` |
| P1-8 | Installer and archive finder (`.msi`, `.exe` setups, `.iso`, `.zip` in Downloads and Desktop) as Review | Mole installer |
| P1-9 | Package-manager caches via their own commands when the tool is installed (`npm cache clean`, `pnpm store prune`, `dotnet nuget locals`, `go clean -cache`, pip, Gradle), with directory fallback as Review | Mole purge and clean, npkill profiles |
| P1-10 | Locked-file insight: show which process holds a file (Restart Manager) | BCU, Microsoft Restart Manager |
| P1-11 | Restore point before uninstall and HKLM startup changes, respecting the 24-hour limit | Sophia Script, BCU backups |
| P1-12 | Duplicate presets (saved include/exclude sets) and a side-by-side image preview | Krokiet |
| P1-13 | Program Files and ProgramData orphan folder finder (no matching Uninstall entry), Review only | BCU |
| P1-14 | Show Windows-registered VolumeCaches handlers read-only, with a "run Disk Cleanup" shortcut | cleanmgr |
| P1-15 | CLI companion with `--json` and `--dry-run` sharing the same core crate | Mole, dust, gdu |
| P1-16 | Cloud-only files report (how much is online-only vs local) without hydrating | Storage Sense, WinDirStat cloud safeguards |

### P2

| # | Feature | Inspired by |
|---|---|---|
| P2-1 | MFT fast scan when elevated on local NTFS, walker fallback | WizTree, TreeSize, WinDirStat |
| P2-2 | Similar images (perceptual hash plus BK-tree) | Czkawka |
| P2-3 | Replace duplicates with hard links (same volume, NTFS only, expert mode) | Czkawka, WinDirStat |
| P2-4 | Broken files and bad-extension finder (Review only) | Czkawka |
| P2-5 | NTFS compression suggestion for cold folders (`compact`), never CompactOS by default | WinDirStat, TreeSize |
| P2-6 | Optional import of a user-supplied Winapp2.ini (not bundled), mapped to Review risk | BleachBit, Winapp2 |
| P2-7 | Scheduled scan with a notification only (never auto-delete) | Storage Sense, Wise Disk Cleaner |
| P2-8 | Live filesystem monitor to refresh the treemap | WinDirStat |
| P2-9 | Sunburst or flame-graph alternative visualizations | WinDirStat 2.x |
| P2-10 | Community rule packs (separate repo, MIT-licensed, schema-validated, reviewed) | Winapp2, BleachBit CleanerML |

---

## 10. License notes for an MIT project

| Source | License | What we may do |
|---|---|---|
| BleachBit, Mole, WinDirStat, Optimizer, Krokiet | GPL-3.0 | Read for ideas. Do not copy code or cleaner definitions |
| QDirStat, K4DirStat | GPL-2.0 | Same as above |
| Winapp2.ini | CC-BY-SA-4.0 | Do not bundle. Optional user import only, after legal review |
| Pearcleaner | Apache-2.0 + Commons Clause | Ideas only (Commons Clause is not OSI open source) |
| Czkawka core/CLI, dua, gdu, ncdu, kondo, npkill, trash-rs, jwalk, Win11Debloat, Sophia Script | MIT | Reuse allowed with the copyright notice kept |
| dust, pdu, BCU | Apache-2.0 | Reuse allowed with NOTICE and attribution; Apache-2.0 code can be included in an MIT project as a dependency |
| diskus | MIT or Apache-2.0 | Reuse allowed |
| WizTree, SpaceSniffer, TreeSize, CCleaner, Wise, PrivaZer | Proprietary | Feature ideas only |

The marker-file table in section 3.6 is a list of facts about build tools, not creative expression, but we will still write our own table and credit kondo and npkill in the docs.

---

## 11. Open questions and unverified items

- Mole's license is GPL-3.0 on the current main branch. Older releases may have used a different license; this does not matter for us because we copy nothing.
- BCU confidence-level names and scoring factors come from a third-party wiki. Confirm in the BCU app or source before citing them in our docs.
- The StartupApproved byte layout (`02`/`03` prefix) comes from secondary sources. Verify on Windows 11 24H2/25H2 before writing to it.
- Behavior of `Delete-DeliveryOptimizationCache` (admin requirement, effect of `-Force`). The Microsoft page is a stub.
- Whether hashing reads always hydrate cloud placeholders, and whether attribute checks alone are enough. Test on a real OneDrive folder.
- The exact cost of per-file hard-link detection on Windows (opening handles), and whether to make it an "accurate mode".
- Removable and network volumes without a Recycle Bin: confirm the behavior with `IFileOperation` and the `trash` crate.
- Whether Fast Startup depends on the hibernation file (commonly stated; Microsoft's page only mentions hybrid sleep).
- Whether WinDirStat 2.x uses MFT reading or another accelerated NTFS method. Its README only says "accelerated when elevated".

---

## Sources

Projects:

- BleachBit: https://github.com/bleachbit/bleachbit, CleanerML docs https://docs.bleachbit.org/cml/cleanerml.html, preferences https://docs.bleachbit.org/doc/preferences.html, expert mode https://docs.bleachbit.org/doc/expert-mode.html, shredding https://docs.bleachbit.org/doc/shred-files-and-wipe-disks.html, 6.0.0 notes https://www.bleachbit.org/news/bleachbit-600
- Winapp2: https://github.com/MoscaDotTo/Winapp2
- Czkawka: https://github.com/qarmin/czkawka, https://github.com/qarmin/czkawka/blob/master/instructions/Instruction.md, https://github.com/qarmin/czkawka/blob/master/instructions/Instruction_Core.md, https://github.com/qarmin/czkawka/blob/master/instructions/Instruction_Krokiet.md
- WinDirStat: https://github.com/windirstat/windirstat
- WizTree: https://diskanalyzer.com/
- SpaceSniffer (secondary): https://www.techspot.com/downloads/5365-space-sniffer.html, https://www.rarst.net/software/spacesniffer/
- QDirStat: https://github.com/shundhammer/qdirstat
- K4DirStat: https://github.com/jeromerobert/k4dirstat
- TreeSize (secondary): https://www.jam-software.com/treesize/features.shtml, https://www.jam-software.com/treesize_free/changes.shtml
- dust: https://github.com/bootandy/dust
- dua-cli: https://github.com/Byron/dua-cli
- gdu: https://github.com/dundee/gdu
- ncdu: https://dev.yorhel.nl/ncdu
- diskus: https://github.com/sharkdp/diskus
- pdu: https://github.com/KSXGitHub/parallel-disk-usage
- jwalk: https://github.com/Byron/jwalk
- trash-rs: https://github.com/Byron/trash-rs
- npkill: https://github.com/voidcosmos/npkill, profiles https://github.com/voidcosmos/npkill/blob/main/docs/profiles.md
- kondo: https://github.com/tbillington/kondo, https://github.com/tbillington/kondo/blob/master/kondo-lib/src/lib.rs
- cargo-sweep: https://github.com/holmgr/cargo-sweep
- Mole: https://github.com/tw93/Mole
- Pearcleaner: https://github.com/alienator88/Pearcleaner
- Bulk Crap Uninstaller: https://github.com/Klocman/Bulk-Crap-Uninstaller, https://www.bcuninstaller.com/, third-party wiki (secondary) https://mintlify.wiki/Klocman/Bulk-Crap-Uninstaller/guide/junk-removal
- Optimizer: https://github.com/hellzerg/optimizer
- Win11Debloat: https://github.com/Raphire/Win11Debloat
- Sophia Script: https://github.com/farag2/Sophia-Script-for-Windows

Microsoft:

- Disk Cleanup handlers: https://learn.microsoft.com/en-us/windows/win32/lwef/disk-cleanup
- Clean up WinSxS: https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/clean-up-the-winsxs-folder
- Actual size of WinSxS: https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/determine-the-actual-size-of-the-winsxs-folder
- Storage Sense: https://support.microsoft.com/en-us/windows/manage-drive-space-with-storage-sense-654f6ada-7bfc-45e5-966b-e24aded96ad5
- Delete previous version of Windows: https://support.microsoft.com/en-US/Windows/Deployment/Install-Upgrade/delete-your-previous-version-of-windows
- Hibernation: https://learn.microsoft.com/en-us/troubleshoot/windows-client/setup-upgrade-and-drivers/disable-and-re-enable-hibernation
- Delete-DeliveryOptimizationCache: https://learn.microsoft.com/en-us/powershell/module/deliveryoptimization/delete-deliveryoptimizationcache
- Checkpoint-Computer: https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/checkpoint-computer
- Restart Manager: https://learn.microsoft.com/en-us/windows/win32/rstmgr/about-restart-manager
- SHFILEOPSTRUCT flags: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shfileopstructa
- Uninstall registry key: https://learn.microsoft.com/en-us/windows/win32/msi/uninstall-registry-key
- Registry cleaner support policy (KB 2563254): https://support.microsoft.com/en-us/topic/2563254
- Rust `DirEntry` docs: https://doc.rust-lang.org/std/fs/struct.DirEntry.html

Press and secondary:

- CCleaner 2017 compromise: https://thehackernews.com/2018/04/ccleaner-malware-attack.html, https://techcrunch.com/2017/09/21/ccleaner-supply-chain-malware-targeted-tech-giants/
- CCleaner 5.45 Active Monitoring: https://www.csoonline.com/article/565979/avast-pulls-ccleaner-version-that-lacked-privacy-options-after-backlash.html
- CCleaner PUA flag 2020: https://www.techradar.com/news/windows-10-is-warning-users-not-to-install-ccleaner, https://pureinfotech.com/microsoft-defender-antivirus-flags-ccleaner-as-unwanted-app/
- Disk Cleanup Downloads option removed: https://www.bleepingcomputer.com/news/microsoft/microsoft-to-remove-downloads-folder-from-disk-cleanup/, https://www.ghacks.net/2019/11/09/microsoft-will-remove-option-to-clear-downloads-folder-using-disk-cleanup
- Windows Installer folder: https://www.advancedinstaller.com/do-not-delete-windows-installer-folder.html
- ESD files and Reset this PC: https://www.howtogeek.com/266337/what-should-i-remove-in-disk-cleanup-on-windows/
- Cleanup recommendations: https://pureinfotech.com/free-up-space-cleanup-recommendations-windows-11/
- Delivery Optimization cache: https://2pintsoftware.com/news/details/windows-delivery-optimization-content-gets-sticky-in-1903
- Wise Disk Cleaner: https://mspoweruser.com/wise-disk-cleaner-review/
- PrivaZer: https://www.ghacks.net/?p=66129, https://betanews.com/article/privazer-is-one-of-the-most-powerful-free-privacy-tools-weve-ever-seen/
- Microsoft PC Manager: https://lazyadmin.nl/it/microsoft-pc-manager/
- Squarified treemaps paper: https://www.win.tue.nl/~vanwijk/stm.pdf
- NTFS reader crates: https://docs.rs/crate/ntfs-reader/0.4.6, https://docs.rs/usn-journal-rs
