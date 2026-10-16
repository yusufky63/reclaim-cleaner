# Rule format

Cleanup targets are data. Two files ship inside the app:

- `rules/targets.json`: fixed locations such as caches, temp folders, logs and dumps.
- `rules/artifacts.json`: build and dependency folders inside projects, recognised by marker files.

Both are embedded at build time. `cargo test -p reclaim-core` parses them and checks every path against the protected-path guard, so a rule that points at a protected folder fails CI.

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

Put a JSON array of target rules in `%APPDATA%\ReClaim\rules\*.json` (or `data\rules` next to the exe in portable mode). They appear with a "Your rule" badge. User rules are validated more strictly: they cannot set `admin`, cannot run commands and cannot reuse a built-in id. Invalid files are listed in Settings and ignored.
