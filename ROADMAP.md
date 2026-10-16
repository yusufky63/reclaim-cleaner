# Roadmap

What is planned after 0.1.0, roughly in order. The research behind each item is in [docs/research.md](docs/research.md). Suggestions are welcome as issues.

The full plan with sizes and reasons is in [docs/research/README.md](docs/research/README.md).

## Next

- **Hidden space explainer**: why used space and scanned space differ (shadow copies, reserved storage, pagefile, hiberfil, MFT, unreadable folders, WSL and Docker disks).
- **"Analyze with ReClaim"** in the Explorer context menu, plus a path argument.
- **What is locking this file**, through the Restart Manager; graceful close only.
- **Steam libraries on every drive**, with shader caches named after their games.
- **Leftover scan after uninstall** with confidence levels.
- **Export** scan results to CSV and JSON; restore point and shadow storage panel.
- **Code signing** through the SignPath Foundation program, then **winget and Scoop** packages.

## Later

- **Fast NTFS scan** by reading the MFT through the elevated helper, a faster non-admin walker, and **instant search** with Everything-style syntax (`ext:`, `size:`, `dm:`, `type:`).
- **Scan snapshots** and "what grew since last time", kept current by the USN journal while the app is open.
- **Processes, services and scheduled tasks**, read-only first, with signatures; changes later and only through the helper.
- **Everything an app owns** (folders, services, tasks, startup entries) and a window hunter.
- **Project finder** from VS Code and JetBrains recents; **cleanup basket** across pages.
- **WSL and Docker disk compaction wizard**; **relocation guide** for model and package stores.
- **Similar images**, **command-line interface**, **more languages**.

## Separate project

- **Deleted-file recovery** as its own app and repository, sharing an NTFS crate with ReClaim. See [docs/research/recovery-tool.md](docs/research/recovery-tool.md).

## Not planned

- Registry cleaning, debloating, privacy or performance tweaks.
- Scheduled or automatic deletion, background monitoring.
- File shredding or free-space wiping.
- Telemetry, accounts, ads or bundled offers.
