# Roadmap

What is planned after 0.1.0, roughly in order. The research behind each item is in [docs/research.md](docs/research.md). Suggestions are welcome as issues.

## Next

- **Code signing** through the SignPath Foundation program, so SmartScreen stops warning.
- **winget and Scoop** packages after the first published release.
- **Steam libraries from the registry and `libraryfolders.vdf`**, so shader caches on other drives are found too (today only the default Steam folder is checked).
- **Leftover scan after uninstall** with confidence levels: AppData and ProgramData folders named after the app or publisher, preselecting only exact matches.
- **Scan snapshots**: reopen the last space map instantly and compare two scans to see what grew.

## Later

- **Fast NTFS scan** by reading the MFT when running as administrator, with the current walker as the fallback.
- **File age view**: space by last-modified year and month.
- **Filter syntax** in the space map (`*.iso >1GB older:1y`).
- **Similar images** in the duplicate finder.
- **Command-line interface** (`reclaim scan`, `reclaim clean --dry-run`) on top of the same engine crate.
- **More languages.** UI strings are in `src/i18n`; rule texts carry their own translations.

## Not planned

- Registry cleaning, debloating, privacy or performance tweaks.
- Scheduled or automatic deletion, background monitoring.
- File shredding or free-space wiping.
- Telemetry, accounts, ads or bundled offers.
