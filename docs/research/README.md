# Research, round 2 (October 2026)

Round 1 ([../research.md](../research.md)) shaped 0.1.0. This round asked: what do people still miss in cleaners and disk analyzers, how do WizTree-class speed and Everything-class search work, which Windows integrations are realistic, which popular apps keep caches we do not know yet, and how a deleted-file recovery tool should be built. The detailed reports are in this folder; this page is the summary and the plan that follows from them.

| Report | What it covers |
|---|---|
| [feature-gaps.md](feature-gaps.md) | Feature matrices of about 30 tools, 13 recurring user pain points with links, 37 candidate features ranked by value and effort, and a do-not-add list |
| [windows-integration.md](windows-integration.md) | Explorer context menu (Windows 10 and 11), "what is locking this file", services and scheduled tasks, processes, app ownership and a Revo-style window hunter, project finder |
| [fast-scan-and-search.md](fast-scan-and-search.md) | MFT reading, USN journal, a faster non-admin walker, an instant search index, helper-to-UI transport, phased plan |
| [cache-locations.md](cache-locations.md) | The second batch of 115 cleanup rules (now in `rules/targets.json`), exclusions and sources |
| [recovery-tool.md](recovery-tool.md) | Deleted-file recovery: landscape, techniques, honest limits, architecture and plan for a separate app |
| [linux.md](linux.md) | The Linux port: protected paths, what a scan may enter, safe deletes, user cache locations, system areas as report rules, hidden space, elevation |

Claims the reports could not verify are marked in place. Reddit could not be read directly (blocked for automated tools); its threads appear only as search-result titles.

## What changed already (0.2.0 work)

These came straight out of the research and user feedback and are implemented:

- **AI models page.** Hugging Face cache, LM Studio, Ollama (removed with `ollama rm`, because layers are shared) and ComfyUI in one list, with cross-tool duplicates detected by name, size and edge hashes.
- **AI assistants page.** What Claude Code, Codex and Gemini CLI keep on disk (sessions per project, checkpoints, logs, caches). Credentials, settings, skills and plugins are never listed.
- **Downloads page.** Type and age breakdown, thumbnails, and browser re-downloads (`file (1).pdf`) flagged when the original has the same size.
- **Real app icons** in Apps and Startup, and file icons in Large files and Downloads, through `IShellItemImageFactory`.
- **Leftover uninstall entries.** Apps whose uninstaller is gone (or that Windows Installer no longer knows) are flagged and can be removed from the list after a `.reg` backup.
- **115 more cleanup rules** (233 in total) and a new `exclude` field. `%TEMP%` cleaning now keeps Blender, Krita and Office autosave and recovery files, which the research found at risk.
- **Themed dropdowns.** The native select popup ignored the dark theme in WebView2.
- **The first four "Next release" items below**: hidden space explainer, "Analyze with ReClaim" in Explorer, "What is using this?", and Steam libraries with per-game shader caches. Two findings changed the plan on the way: the Restart Manager takes about a tenth of a second per file, so folders are checked with `FileProcessIdsUsingFileInformation` in parallel under a time budget (which also finds terminals that use the folder as their working folder); and the storage reserve size comes from `GetDiskSpaceInformationW` without admin rights.

## Plan

Ordered by value for effort. Sizes: S up to 3 days, M up to 2 weeks, L longer.

### Next release

| Feature | Size | Why |
|---|---|---|
| Hidden space explainer: used space minus scanned space, split into shadow copies, reserved storage, pagefile/hiberfil/swapfile, MFT, unreadable folders, WSL/Docker disks | M | The most common question in every forum: "C: is full but the folders do not add up" |
| "Analyze with ReClaim" in the Explorer context menu, plus a path argument | M | WizTree, TreeSize and WinDirStat all have it. Classic verbs work on Windows 10 and 11 ("Show more options"); the Windows 11 top-level menu needs a signed package, so it waits for code signing |
| What is locking this file (Restart Manager) | M | One of the most asked questions; we show the process and offer a graceful close, never force-close handles |
| Steam libraries on every drive, shader caches with game names | S | Single games reach tens of GB of shader cache |
| Export scan results to CSV and JSON | S | Standard in every competitor |
| Restore points and shadow storage panel | S | Frequent root cause of "missing" space; actions go through Windows' own screens |
| Leftover scan after uninstall with confidence levels | M | In progress in a separate branch |

### Fast scan and instant search

| Phase | Size | Notes |
|---|---|---|
| Release-build timings, fewer allocations per file | S | The 108 s figure was a debug build |
| Non-admin walker on `NtQueryDirectoryFile` with large buffers, allocated sizes, hard links counted once | M | Also the fallback for ReFS, FAT and network drives |
| Elevated MFT reader through the helper, streamed over a locked-down named pipe | L | WizTree-class speed (estimated 4 to 7 s for this C:) |
| Search index with Everything-style syntax (`ext:`, `size:`, `dm:`, `path:`, `type:`, `dupe:`) and virtualized results | L | Also gives "search by file type" across the whole disk |
| Snapshot cache and live updates from the unprivileged USN journal; "what grew since last scan" | M to L | No background service; the index refreshes while the app is open |

### System insight (read first, change second)

| Feature | Size | Notes |
|---|---|---|
| Processes view with path, publisher, memory and the installed app it belongs to | M | Documented APIs only |
| Services and scheduled tasks, read-only with signature checks | M | Changing them comes later, only through the helper, never for Microsoft-signed or driver services, never as presets |
| Everything an app owns: install folder, data folders, services, tasks, startup entries, file associations; plus a window hunter | L | Confidence-scored; only explicit evidence is preselected |
| Project finder from VS Code and JetBrains recents, with git facts | M | Extends the Developer page |
| One cleanup basket across pages | M | One plan, one confirmation |
| WSL and Docker disk compaction wizard | M to L | Strong guards; official commands only |
| Relocation guide for stores that can move (OLLAMA_MODELS, HF_HOME, package caches, Docker disk) | M | Generates the commands, does not move data itself |

### Deliberately not planned

Registry cleaning, RAM or "performance" boosters, driver updaters, shredding and free-space wiping, moving programs with junctions, forcing locked handles closed, killing processes, scheduled deletion, telemetry. See section 6 of [feature-gaps.md](feature-gaps.md) for the reasoning.

## Deleted-file recovery: a separate app

Recovery reads raw disks as administrator; the cleaner deletes files as a normal user. Mixing the two risk profiles in one app would make both harder to trust, so recovery should be its own MIT repository that shares a small NTFS crate with ReClaim.

- **MVP (about 11 to 15 weeks):**
  - a "stop writing to this drive" first screen and a per-drive outlook (SSD TRIM, system drive, shadow copies);
  - easy wins first: Recycle Bin, existing shadow copies, File History, app autosaves;
  - NTFS undelete from the MFT;
  - FAT32 and exFAT undelete for USB sticks and SD cards;
  - a small validated carver for photos, video, PDF and Office files;
  - imaging before recovery for removable media.
- **Hard rules:**
  - the source is opened read-only and is never written to;
  - output is refused on the same physical disk;
  - every file gets a recoverability state with a reason, never "100%".
- **Naming:** "ReclaiMe" is an established commercial recovery product, so "ReClaim Recover" risks confusion. Pick a distinct name after a trademark search.

Details, including the open-source landscape (TestDisk/PhotoRec, winfr, The Sleuth Kit, and eight new 2025–2026 projects), licences to avoid and the test strategy, are in [recovery-tool.md](recovery-tool.md).
