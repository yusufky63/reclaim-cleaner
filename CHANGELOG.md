# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
