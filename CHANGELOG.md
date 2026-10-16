# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

### Fixed

- Dropdowns showed light popups in dark mode.
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

[Unreleased]: https://github.com/yusufky63/reclaim-cleaner/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yusufky63/reclaim-cleaner/releases/tag/v0.1.0
