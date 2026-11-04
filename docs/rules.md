# Rule format

Cleanup targets are data. Three files ship with the source, and each build embeds the ones for its platform:

- `rules/targets.json`: fixed locations on Windows, such as caches, temp folders, logs and dumps.
- `rules/linux/targets.json`: the same on Linux (see [Linux rules](#linux-rules)).
- `rules/artifacts.json`: build and dependency folders inside projects, recognised by marker files. Shared by all platforms.

`cargo test -p reclaim-core` parses them and checks every path against the protected-path guard, so a rule that points at a protected folder fails CI.

## Targets

```json
{
  "id": "vscode-cache",
  "group": "apps",
  "name": "VS Code cache",
  "nameTr": "VS Code önbelleği",
  "description": "HTTP, code and GPU caches of VS Code; rebuilt on the next start. The editor must be closed.",
  "descriptionTr": "VS Code'un HTTP, kod ve GPU önbellekleri; bir sonraki açılışta yeniden oluşur. Düzenleyici kapalı olmalı.",
  "paths": ["%APPDATA%\\Code\\Cache", "%APPDATA%\\Code\\Code Cache"],
  "mode": "contents",
  "minAgeHours": null,
  "risk": "safe",
  "admin": false,
  "closeApps": ["Code.exe"],
  "verified": true,
  "source": "https://..."
}
```

| Field | Meaning |
|---|---|
| `id` | Unique, kebab-case. Never reuse an id for a different location. |
| `group` | `system`, `browser`, `apps`, `dev`, `games`, `ai` or `media`. |
| `name`, `nameTr` | Short label in English and Turkish. |
| `description`, `descriptionTr` | One sentence: what it is and what happens after deletion. |
| `paths` | Absolute paths that start with an allowed variable: `%LOCALAPPDATA%`, `%APPDATA%`, `%USERPROFILE%`, `%TEMP%`, `%WINDIR%`, `%PROGRAMDATA%`, `%SYSTEMDRIVE%`. Two more stand for folders found at run time: `%STEAM%` (the Steam folder, from `HKCU\Software\Valve\Steam`) and `%STEAMLIBRARY%` (every Steam library on every drive, from Steam's `libraryfolders.vdf`); a rule with them expands once per folder and to nothing when Steam is not installed, and rules that need admin rights cannot use them. `*` and `?` match inside one segment (`User Data\*\Cache`, `webcache*`). The first segment after the variable must be a fixed name. No `..`, no `**`. |
| `mode` | `contents`: delete everything inside each path and keep the folder. `folder`: delete each path itself. `files`: delete files directly inside each path that match `include` (not recursive). `report`: measure only; cleaning happens through `command`. |
| `include` | File name globs for `files` mode, such as `["*.dmp"]`. |
| `exclude` | File or folder name globs kept wherever they appear inside the paths, such as `["*.blend", "gradio"]` for autosaves and outputs that apps write into `%TEMP%`. |
| `minAgeHours` | Skip files modified more recently than this. Use it for shared temp folders. |
| `risk` | `safe` (rebuilds by itself, preselected), `review` (shown, not preselected), `caution` (hidden unless enabled in Settings). |
| `admin` | `true` when the location needs administrator rights. Those rules are cleaned by the elevated helper. |
| `closeApps` | Executable names. While one runs, the rule is blocked. |
| `command` | The tool's own cleanup command (`npm cache clean --force`), offered as "Run official command". `api:SHEmptyRecycleBin...` is reserved for the Recycle Bin. |
| `verified` | `true` when `source` documents the location. Unverified rules show a "Community path" badge. |
| `source` | Link to vendor docs or another reliable reference. |

### Choosing a risk level

- **safe**: a cache or log the app rebuilds by itself, with nothing the user would miss. Pages load slower once; that is fine.
- **review**: re-downloadable but large or slow to get back (AI models, SDK images, the pnpm store), or useful history (crash dumps, update downloads).
- **caution**: can lose data or hurt performance: Prefetch, Maven repository (manually installed jars), Android virtual devices, `build` folders that may hold source assets.

When in doubt, choose the higher risk.

### What never becomes a rule

- Anything under Documents, Desktop, Downloads, Pictures, Music, Videos or OneDrive as a whole.
- Cookies, browser history, passwords, sessions, Local Storage, IndexedDB.
- `WinSxS`, `Windows\Installer`, driver store, reset images, `hiberfil.sys`, `pagefile.sys`. Use Windows' tools (see the Windows tools page).
- Registry keys.

## Linux rules

`rules/linux/targets.json` uses the same fields with these differences. `docs/research/linux.md` holds the paths, risks and sources behind it.

| Field | On Linux |
|---|---|
| `paths` | Start with `%HOME%`, `%XDG_CACHE%`, `%XDG_CONFIG%`, `%XDG_DATA%` or `%XDG_STATE%` (each falls back to the XDG default, so `%XDG_CACHE%` is usually `~/.cache`), or `%STEAM%` and `%STEAMLIBRARY%`. Separators are `/`. A rule must name a child: `%XDG_CACHE%` alone is refused. Paths are case-sensitive. There are no brace lists, so `{Cache,Code Cache}` is written out as two paths. |
| `closeApps` | Process names without `.exe`, such as `code` or `signal-desktop`. They are compared in lowercase. |
| `admin` | Always `false`. The first Linux version never runs as root. |
| `command` | Shown for copying, not run. It starts with the tool itself, without `sudo`: the scan uses the first word to tell whether this distro has the tool. The description says when root is needed. It may not contain placeholders or shell operators, and may be a listing command (`coredumpctl list`) when the system ages the data out by itself. |
| `verified` | `true` only when `source` is a primary source that names the path. Anything derived from a convention stays `false`, including Electron folder names and most Flatpak and Snap paths. |

### System areas

Package caches, the journal, snaps, Flatpak runtimes and containers are `report` rules with their official command. Nothing below `/var`, `/usr` or `/etc` is ever deleted as files, and no variable reaches those folders, so most of these rules have empty `paths`. Their size is not measured.

### Flatpak and Snap

Inside a sandboxed app's folder the guard allows only its cache, so rules follow the same line:

- Flatpak moves an app's XDG folders into `%HOME%/.var/app/<id>/`. A rule may use `<id>/cache`, never `<id>/config`, `<id>/data` or `<id>/.local`.
- A strictly confined Snap keeps its cache in `%HOME%/snap/<name>/common/.cache`. Everything else there is app data, and numbered revisions would count the same app twice. Classic snaps such as `code` use the real home folder, so they need no extra path.
- Electron apps keep their caches under `config`, so their Flatpak and Snap installs have no rule yet. The same goes for Telegram, whose cache is under `data`.
- When the native path is verified, sandbox paths go in a separate unverified rule (`chrome-sandbox-cache`). Otherwise they share one rule with the native path.

### What never becomes a Linux rule

The list below is enforced by tests in `crates/core/src/rules.rs`.

- `~/.cache`, `~/.config`, `~/.local/share`, `~/.var/app`, `~/snap` as a whole.
- Keys and passwords: `~/.ssh`, `~/.gnupg`, `~/.password-store`, keyrings.
- A whole browser profile. Chromium-based profiles allow only `*/Service Worker/CacheStorage`, at `review`. Firefox profiles are off limits; its cache lives in `%XDG_CACHE%/mozilla`.
- Mail: `~/.thunderbird`, Evolution.
- Telegram's `tdata`, except `user_data*/cache` and `user_data*/media_cache`. Signal's `sql` and `attachments.noindex`.

Proton, Wine, Lutris and Heroic prefixes hold save games that may exist nowhere else, so they are `report` rules at `caution`.

## Artifacts

```json
{
  "id": "rust-target",
  "dirName": "target",
  "markers": ["Cargo.toml"],
  "contains": [],
  "risk": "safe",
  "description": "Cargo build output; rebuilt by cargo build.",
  "descriptionTr": "Cargo derleme çıktısı; cargo build ile yeniden oluşur."
}
```

| Field | Meaning |
|---|---|
| `dirName` | Folder name, globs allowed (`cmake-build-*`). |
| `markers` | Any one of these must exist next to the folder, in the project root. Globs (`*.csproj`) and relative paths (`ProjectSettings/ProjectVersion.txt`) work. Empty means the name alone is proof (`.pytest_cache`). |
| `contains` | Files that must exist inside the folder (`pyvenv.cfg` for `.venv`). |

Rules are tried in file order, so specific owners come before generic ones: a Gradle `build` folder is matched by the Gradle rule before the JavaScript one.

## Your own rules

Put a JSON array of target rules in `%APPDATA%\ReClaim\rules\*.json` (or `data\rules` next to the exe in portable mode). They appear with a "Your rule" badge. User rules are validated more strictly: they cannot set `admin`, cannot run commands and cannot reuse a built-in id. Invalid files are listed in Settings and ignored. On Linux they follow the Linux path format above.
