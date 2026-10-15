# Contributing

Thanks for helping. ReClaim deletes files on real computers, so the bar for changes to the engine and the rules is high, and the bar for everything else is friendly.

## Ground rules

- **Safety beats coverage.** A rule that frees 2 GB for most people but can delete one person's work does not ship. When in doubt, mark it `review` or leave it out.
- **No registry cleaning, tweaks, telemetry, bundled offers or nag screens.** Pull requests that add them will be closed.
- **No code or rule files from GPL projects** (BleachBit, Winapp2, WinDirStat, QDirStat, Mole). Ideas are fine; write your own code and cite your sources.
- **English for code, comments, UI strings and commits.** Every UI string also goes into `src/i18n/tr.ts`; the typecheck fails if a key is missing.

## Setup

Windows 10 or 11 with Rust (stable, MSVC), Node.js 22+, pnpm 9 and the Visual Studio C++ build tools.

```bash
pnpm install
pnpm app:dev
```

## Before you open a pull request

All of these must pass; CI runs the same set.

```bash
pnpm typecheck
pnpm lint
pnpm test
pnpm build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- UI changes: check light and dark theme and the minimum window width (980 px). Attach screenshots.
- Engine changes: add a test that uses a temporary folder. Anything that deletes needs a test proving it does not follow junctions and respects exclusions.
- User-visible changes: add a line to `CHANGELOG.md` under "Unreleased".

## Adding or changing a cleanup rule

Read [docs/rules.md](docs/rules.md). In short:

1. Add the rule to `rules/targets.json` (fixed locations) or `rules/artifacts.json` (project build folders).
2. Use only the allowed variables (`%LOCALAPPDATA%`, `%APPDATA%`, `%USERPROFILE%`, `%TEMP%`, `%WINDIR%`, `%PROGRAMDATA%`, `%SYSTEMDRIVE%`).
3. Link a source that documents the location in `source`. Without one, set `"verified": false`.
4. Write both `name`/`description` and `nameTr`/`descriptionTr`. Say what is lost after deletion.
5. `cargo test -p reclaim-core` validates every rule against the protected-path guard.

## Commits and releases

Conventional Commits (`feat(engine): …`, `fix(ui): …`, `rules: …`). Releases are tagged `vX.Y.Z` from `main`; the release workflow builds the installers. See [docs/releasing.md](docs/releasing.md).
