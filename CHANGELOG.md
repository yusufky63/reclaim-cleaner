# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Quick clean from outside the app. Right-click ReClaim on the taskbar (or its pinned Start entry) and choose "Quick clean", open "ReClaim Quick Clean" in the Start menu (added by the setup), or put the same shortcut on the desktop from Settings > Quick clean shortcuts. Each opens the app on the Overview with a fresh scan and the usual quick clean confirmation listing every item; nothing is deleted until you confirm. Behind them is the `--quick-clean` flag, which only opens that confirmation. The setup removes both shortcuts on uninstall.

### Changed

- "Analyze with ReClaim" in Explorer's right-click menu is now on by default for installed copies: it is added the first time the app starts, once, and stays off if you turn it off in Settings. Portable copies still leave the registry alone until you turn it on. On Windows 11 the entry is under "Show more options" (Shift+F10), like other classic entries such as WizTree's.

- Developer page: every project is listed and all of its rebuildable folders are selected by default ("Idle for" starts at "Any project"). "Select idle" narrows the selection to projects untouched for the number of days set in Settings. Folders marked "Review first", such as virtual environments, are still not preselected.
- Project folders are also found when they are named "Projects" in another UI language (Projekte, Proyectos, Projets, Progetti, Projetos, Projekty, Проекты, Проєкти, Proyek, プロジェクト, 프로젝트, 项目), in the user folder and at the root of every fixed drive.

## [0.2.0] - 2026-10-16

### Added

- AI models page: Hugging Face, LM Studio, Ollama and ComfyUI models in one list, with models stored by two tools flagged. Ollama models are removed with `ollama rm`.
- AI assistants page: sessions, checkpoints, logs and caches of Claude Code, Codex and Gemini CLI. Credentials and settings are never listed.
- Downloads page: breakdown by type and age, list and thumbnail grid, browser re-downloads such as `file (1).pdf` flagged.
- Real app icons in Apps and Startup, file icons in Large files and Downloads. Store apps show their package logo and their real size.
- Quick clean on the Overview: one click clears every safe cache, temp file and log, after a single confirmation that lists what is included.
- Leftover uninstall entries (the app is already gone) are flagged and can be removed after a `.reg` backup.
- 115 more cleanup rules (233 in total): Office, OneDrive, Dropbox, Google Drive, Signal, Adobe, Blender, OBS, Plex, Jellyfin, EA app, Ubisoft, GOG, Minecraft, NVIDIA and AMD apps, winget, Scoop, Chocolatey, Docker, Vagrant, more browsers and Electron apps.
- Rules can exclude names. `%TEMP%` cleaning keeps Blender, Krita and Office autosave files.
- Research round 2 and the resulting plan in `docs/research/`.
- 12 more UI languages: German, Spanish, French, Italian, Portuguese (Brazil), Polish, Russian, Ukrainian, Indonesian, Japanese, Korean and Simplified Chinese. Missing strings fall back to English; dates and numbers follow the chosen language.
- Leftover scan after uninstall. Once the app's own uninstaller has finished, folders in AppData, ProgramData and Program Files named after the app or its publisher are listed with a confidence level. Only high-confidence folders (the install folder, an exact name match, `Publisher\App`) are preselected; publisher folders and similar names are left for you to check, and folders another installed app may still use are not listed. Removal goes to the Recycle Bin by default.
- Hidden space on the Space map: when a whole drive is mapped, its used space is reconciled with the map. Restore points and shadow copies (measured once with administrator rights, read only), reserved storage, the page, hibernation and swap files, the NTFS file table, the Recycle Bin, WSL and Docker disks, unreadable folders and online-only cloud files are listed with their size, whether the map counted them, and a link to Windows' own screen. The Overview shows what Windows keeps on the system drive.
- "Analyze with ReClaim" in Explorer's right-click menu on folders, folder backgrounds and drives. It is off by default, per user, and turned on in Settings; on Windows 11 it is under "Show more options". The app accepts `--analyze <folder>`, also when it is already running (as administrator too), and folders selected together are mapped as one selection. The setup removes the entry on uninstall but not on update.
- "What is using this?" in Large files, the Space map and the "in use" lines of clean reports. It lists the apps, background programs, services and Windows processes that hold a file or folder, including programs that run from the folder or have it open as their working folder, and offers a graceful close (like clicking the window's close button) and a re-check. Processes are never ended and handles are never closed.
- Games page: Steam libraries on every drive, installed games with their sizes, and the shader cache of each game named from Steam's manifests. Caches of removed games are preselected; Steam must be closed to delete.

### Changed

- Steam rules (shader cache, logs and dumps, download leftovers, unfinished downloads, client packages) follow the Steam folder and every library listed in Steam's own settings instead of assuming `C:\Program Files (x86)\Steam`. Rules can use `%STEAM%` and `%STEAMLIBRARY%`.
- Pages other than the Overview load on first visit, which keeps the startup bundle small.

### Fixed

- Dropdowns showed light popups in dark mode.
- "Delete the application data" in the setup's uninstaller left settings, history, user rules and `.reg` backups in `%APPDATA%\ReClaim`. It now removes that folder too; with the box unticked, the folder stays.
- Future timestamps on single files no longer make folders look recently modified.
- Items that cannot be moved to the Recycle Bin now show the real reason. A folder that needs administrator rights, such as one under Program Files or ProgramData, reports "Access denied" instead of "In use by another program"; locked files still report "In use".
- A leftover install folder under Program Files could not be removed, because hand-picked deletes may never touch Program Files. A dedicated leftover guard now allows exactly the folders the scan reported, one or two levels below Program Files or ProgramData, when ReClaim runs as administrator.

## [0.1.0] - 2026-10-16

First public version.

### Added

- Overview with free space now and after cleanup, safe and review totals, ranked recommendations and a PowerShell script export.
- Space map: parallel scan of any drive or folder, two-level treemap by category, Cleanup view, file type statistics, delete from the map.
- Junk and caches: 118 built-in rules for Windows, browsers, apps, games, developer tools and AI model caches, with running-app checks and official cleanup commands where they exist.
- Developer page: 42 build-artifact rules detected by project marker files, idle-project filter.
- Large files, duplicate finder (size, quick hash, BLAKE3, hard-link aware) and empty folder finder.
- Apps: desktop and Store apps, uninstall with the vendor uninstaller, leftover folder check.
- Startup apps: enable and disable through StartupApproved, like Task Manager.
- Windows tools: Recycle Bin, hibernation, DISM component cleanup, Delivery Optimization, previous Windows installation, Disk Cleanup, Storage Sense and System Protection shortcuts.
- Elevated helper for admin-only rules, protected-path guard, user exclusions, cleanup history.
- English and Turkish UI, light and dark theme, portable mode.
- NSIS installer (English and Turkish), MSI packages (en-US, tr-TR) and a portable zip.

[Unreleased]: https://github.com/yusufky63/reclaim-cleaner/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/yusufky63/reclaim-cleaner/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/yusufky63/reclaim-cleaner/releases/tag/v0.1.0
