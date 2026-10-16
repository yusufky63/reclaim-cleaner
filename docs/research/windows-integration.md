# Research 2: Windows integration features for ReClaim Cleaner

Scope: the Windows techniques, APIs and crates needed for seven planned features: Explorer context menu integration, "what is locking this file", a services and scheduled tasks view, a running processes view, app ownership (with a window "hunter"), a project finder, and the Tauri and elevated-helper plumbing behind them.

Target: Tauri 2.12 + Rust, `windows` 0.62.2, Windows 10 and 11. The app runs as a normal user. Admin work goes through the existing elevated helper (same exe, started with ShellExecuteEx `runas`, request file with ids only).

Research date: 2026-10-16. The web search budget ran out late in the session, so a few points below could not be checked again and are marked.

Markers:

- **(verified)**: confirmed on Microsoft Learn, in another primary source, or in the crate source in the local cargo registry.
- **(tested)**: run on this machine (Windows 11 Pro build 26200, **not elevated**) with a throwaway script. The tests read data only and printed counts or shapes, never contents.
- **(secondary)**: press or third-party write-ups.
- **(unverified)**: general knowledge I could not confirm. Check before relying on it.

Rust sketches are for orientation only. They have not been compiled. The function names and signatures were checked against `windows-0.62.2/src` in the local registry.

---

## Summary table

| # | Feature | Recommended approach | Admin? | Effort |
|---|---|---|---|---|
| 1 | Explorer context menu | Classic HKCU registry verbs written by the app (opt-in toggle), args forwarded through single-instance. Defer the Windows 11 modern menu (needs a signed identity package plus a COM DLL) | No | M (classic). L (modern, blocked on code signing) |
| 2 | Lock finder | Restart Manager on files, plus "processes whose image is inside this folder". Graceful close with WM_CLOSE only | No for detection | M |
| 3 | Services and tasks | SCM APIs read-only for everyone. Changes through the helper, re-validated there. Windows and Microsoft services read-only. Task Scheduler COM read plus enable/disable for non-Microsoft tasks | Read: no. Change: yes | L (split into 3 M steps) |
| 4 | Processes | Own thin layer: Toolhelp + `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `NtQueryInformationProcess(ProcessCommandLineInformation)` + version info. `sysinfo` 0.39 is the alternative | No (sees about 3/4 of processes) | M |
| 5 | App ownership | Evidence collectors (registry, MSI, Store, services, tasks, Run, firewall, COM, shell ext, App Paths), each yielding explicit or name-based evidence, plus a score with hard blocks. Hunter = cursor polling + `WindowFromPoint` | Mostly no. Prefetch and other users' MSI data need admin | L |
| 6 | Project finder | Extend the existing `artifacts.rs` scan with IDE recent lists (VS Code workspaceStorage, JetBrains, Visual Studio) and cheap git facts (reflog, config). `git` CLI or `gix` only for exact last-commit data | No | M |
| 7 | Tauri plumbing | No new plugins. Rust commands, `opener` reveal from Rust, helper protocol generalized to typed operations | n/a | S to M |

---

## 1. Explorer right-click integration

### Approach

Ship **classic static verbs** under `HKCU\Software\Classes`. They need no admin, no COM and no signing, work on Windows 10 and 11, and can be written and removed by the app itself. Make them **opt-in** in Settings ("Add ReClaim to the Explorer right-click menu"). The installers only remove them on uninstall.

Verbs:

| Verb | Registry class keys | Command line | Selection |
|---|---|---|---|
| Analyze with ReClaim | `Directory\shell`, `Directory\Background\shell`, `Drive\shell` | `"<exe>" --analyze "%V"` (Drive: `%V` unquoted, see pitfalls) | `MultiSelectModel=Single` |
| Find what is locking this | `*\shell`, `Directory\shell` | `"<exe>" --who-locks "%1"` | `Player`, batched in the app |
| Delete safely with ReClaim | `AllFilesystemObjects\shell` (files and folders) | `"<exe>" --delete-safely "%1"` | `Player`, batched in the app |

Use `Directory`, not `Folder`. `Folder` also matches virtual folders such as libraries and Control Panel, which have no file system path (verified that `Directory\Background` and `Drive` are predefined objects that accept verbs, from Learn's predefined shell objects table; the exact table wording was not re-read, unverified).

Optional: group the three under one cascading entry "ReClaim" using `MUIVerb` + `ExtendedSubCommandsKey`. Learn says this works under `HKCU\Software\Classes` without elevation (verified). The `SubCommands` + `CommandStore` route needs HKLM, so avoid it (verified).

### Windows 11 modern menu (the top-level one)

- Windows 11 puts registry verbs and `IContextMenu` handlers under **"Show more options"** (or Shift+F10). Only **`IExplorerCommand` handlers registered by an app with package identity** appear in the new top-level menu (verified, Windows blog 2021).
- An unpackaged app gets identity through a **package with external location** (sparse package). Requirements (verified, Learn):
  - An AppxManifest with `uap10:AllowExternalContent=true`, `runFullTrust` and `unvirtualizedResources`, an `Application` element with `uap10:TrustLevel="mediumIL"` and `uap10:RuntimeBehavior="win32App"`, and `AppListEntry="none"`. MinVersion `10.0.19041.0`: AllowExternalContent exists from build 19041 (Windows 10 2004).
  - The exe must embed an application manifest with an `<msix publisher=... packageName=... applicationId=...>` element matching the package. Tauri supports a custom manifest through `tauri_build::WindowsAttributes::app_manifest` (verified in tauri-build 2.7.3 source). The custom manifest must keep what Tauri's default one contains (common controls v6, DPI awareness) (unverified that nothing else is needed).
  - Package it with `MakeAppx pack /nv`. **Sign it with a certificate trusted on the target machine.** Self-signed only works after the user imports the public cert into Trusted People. Learn says a production certificate is required for end-user machines.
  - Register per user, no admin: `Add-AppxPackage -Path <msix> -ExternalLocation <installdir>` or `PackageManager.AddPackageByUriAsync` with `AddPackageOptions.ExternalLocationUri`. Unregister with `Remove-AppxPackage`. Per-machine needs `Add-AppxProvisionedPackage` (admin).
  - Manifest extensions: `com:Extension Category="windows.comServer"` with `com:SurrogateServer` / `com:Class Id Path ThreadingModel="STA"`, and `desktop4:Extension Category="windows.fileExplorerContextMenus"` with `desktop5:ItemType Type="Directory" | "Directory\Background" | "*"` and `desktop5:Verb Id Clsid` (verified: desktop4 from 1803, desktop5 ItemType from 1809. The `Directory\Background` value comes from a Learn page seen only as a search excerpt, secondary).
  - The handler is an in-proc COM DLL implementing `IExplorerCommand`, loaded in a `dllhost` surrogate. In Rust this means a separate `cdylib` crate with `DllGetClassObject`, `DllCanUnloadNow`, an `IClassFactory`, and `#[implement(IExplorerCommand)]`. `windows-rs` 0.62 exposes `IExplorerCommand_Impl` (verified).
- **Unsigned packages** (OID `2.25.311729368913984317654407730594956997722` in the Publisher) exist since Windows 11. They need admin when the package has executable content, and Microsoft says not to use them for distribution (verified). Not a solution.
- **Code signing is the blocker.** Options are a commercial OV certificate, Azure Artifact Signing (formerly Trusted Signing), or SignPath's free OSS program. Eligibility for an individual outside the US or Canada is **unverified** (the search budget ran out).
- **Windows 11 26H2 change (secondary, Insider "Experimental" build 26340.9212, August 2026):**
  - The menu is being redesigned: third-party entries go into an "App extensions" submenu, with a "Customize menu" page under Settings > Personalization.
  - A Microsoft Insider lead posted that "'Show more options' is off by default. It can be turned on in settings if a user has legacy extensions."
  - If this ships as reported (26H2 is expected around October 2026), **registry verbs become invisible by default on new Windows 11 builds** until the user flips a setting. Whether Shift+F10 still opens the legacy menu is unknown (unverified).
  - Microsoft said it will publish migration guidance. Re-check on the first 26H2 release build.

**Recommendation:** ship classic verbs now (M). Say in the Settings text and docs that on Windows 11 they are under "Show more options", and possibly behind a Settings switch on newer builds. Plan the modern handler (L) only once the project has a trusted signing certificate. It also adds a COM DLL that runs in an Explorer surrogate, so it needs its own crash and hang testing.

Portable mode cannot use the sparse package route in practice: the package is tied to an external location path, and registration needs a trusted signature. Classic verbs work fine for portable use.

### Registration from installers and portable mode

- **Single source of truth: the app.** A Settings toggle calls `register(exe)` or `unregister()`. On every start, if the toggle is on and the stored command points at a different exe (portable copy moved, reinstall into another folder), rewrite the keys, or turn the toggle off and say so.
- **NSIS** (`installMode: currentUser`, as in this repo):
  - Add `bundle.windows.nsis.installerHooks` with a `NSIS_HOOK_PREUNINSTALL` macro that runs `DeleteRegKey HKCU "Software\Classes\Directory\shell\ReClaim.Analyze"` and the other verb keys (verified hook names, Tauri docs).
  - **Guard it with `${If} $UpdateMode <> 1`.** Tauri's NSIS uninstaller also runs during updates (`/UPDATE`), and deleting the keys then would silently turn the feature off after every update (verified in the Tauri `installer.nsi` source).
  - Optional: an install-page checkbox needs NSIS page customization, which Tauri's template makes harder. The Settings toggle is simpler.
- **MSI** (Tauri's WiX template is `InstallScope="perMachine"`, verified in the source):
  - HKCU keys written by the app at runtime are per user. A WiX fragment (`fragmentPaths` + `componentRefs`) can remove them on uninstall, but only for the user who runs the uninstall.
  - Other users keep dead verbs that point at a missing exe. Mitigation: the verb command only runs our exe, so a dead verb just fails to launch, which is harmless. Document it.
  - Do not write HKLM verbs from the MSI unless you want machine-wide integration that every user sees.
- **Portable:** the same toggle. Show a "Remove Explorer integration" button and document the manual cleanup.
- After writing or deleting, call `SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None)`. This is optional for static verbs, but cheap.

### Argument forwarding (single instance)

- Explorer starts a new `reclaim-cleaner.exe` per invocation. `tauri-plugin-single-instance` 2.5.4 (pinned in this repo) detects the running copy through a named mutex.
  - It sends `cwd\0arg0\0arg1...` with `WM_COPYDATA` (`SendMessageTimeoutW`, 10 s) to a hidden window with class `{identifier}-sic` and title `{identifier}-siw`, then exits.
  - The running instance gets `(app, argv, cwd)` in the init closure (verified in the plugin source).
  - The current callback in `src-tauri/src/lib.rs:804` ignores argv. Parse it there.
- **Pitfall, UIPI (verified from source, behavior unverified at runtime):**
  - The app has a "Restart as administrator" mode. When the running instance is **elevated** and Explorer launches a medium-integrity second copy, UIPI blocks `WM_COPYDATA` from lower to higher integrity.
  - The plugin calls no `ChangeWindowMessageFilterEx`, so the send fails, the second copy still exits, and the request is **silently lost**.
  - Fix: after setup, find the plugin's window (`FindWindowW` with the class name above) and call `ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None)`.
  - Then **treat forwarded args as untrusted input**: they may now come from any medium-integrity process. Only map them to "open view X for path P". Never run a destructive action from argv without an in-app confirmation.
  - The class name is an internal detail of the plugin. Re-check it on every plugin upgrade.
- **Multi-select:** with `Player`, Explorer starts up to 100 processes for a legacy verb, and `Document` up to 15 (verified). Each one forwards to the first instance. Debounce about 300 ms in the callback and merge paths into one request.
- **Cold start:** when no instance is running, the args arrive in `std::env::args()` in `main.rs`. Route both paths through the same `handle_shell_args()`.

### Pitfalls

- **Quoting a drive root:**
  - `"%V"` for a drive or a drive-root background gives `"C:\"`. MSVC argv rules treat `\"` as a literal quote, so the program receives `C:"` (unverified that Explorer passes it exactly like this, but the parsing rule is standard).
  - Use `%V` unquoted for `Drive\shell` (drive roots contain no spaces).
  - Also normalize in the app: strip a trailing `"`, and if the result is `X:`, append `\`.
- `%V` vs `%1`: `%V` is the "verb target" and works for backgrounds. `%1` is the selected item. Use `%V` for `Directory\Background`.
- Validate every path from argv: absolute, exists, a file system path (not `::{CLSID}`), not a UNC path unless supported. Canonicalize with care: `std::fs::canonicalize` returns `\\?\` paths.
- Icon value: `C:\path\reclaim-cleaner.exe,0`.
- Do not use `IContextMenu` shell extensions (in-proc DLL in Explorer). They are slow, crash-prone, and also hidden under "Show more options" on Windows 11.

### Rust sketch (classic verbs, `winreg` is already a dependency)

```rust
use std::path::Path;
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

/// (class key, verb id, label, args, multi-select model)
const VERBS: &[(&str, &str, &str, &str, &str)] = &[
    (r"Directory\shell", "ReClaim.Analyze", "Analyze with ReClaim", r#"--analyze "%V""#, "Single"),
    (r"Directory\Background\shell", "ReClaim.Analyze", "Analyze with ReClaim", r#"--analyze "%V""#, "Single"),
    // `"C:\"` would reach argv as `C:"`, and drive roots never contain spaces.
    (r"Drive\shell", "ReClaim.Analyze", "Analyze with ReClaim", "--analyze %V", "Single"),
    (r"*\shell", "ReClaim.WhoLocks", "Find what is locking this", r#"--who-locks "%1""#, "Player"),
    (r"Directory\shell", "ReClaim.WhoLocks", "Find what is locking this", r#"--who-locks "%1""#, "Player"),
    (r"AllFilesystemObjects\shell", "ReClaim.DeleteSafely", "Delete safely with ReClaim", r#"--delete-safely "%1""#, "Player"),
];

pub fn register(exe: &Path) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for (class, id, label, args, model) in VERBS {
        let (k, _) = hkcu.create_subkey(format!(r"Software\Classes\{class}\{id}"))?;
        k.set_value("MUIVerb", label)?;
        k.set_value("Icon", &format!("{},0", exe.display()))?;
        k.set_value("MultiSelectModel", model)?;
        let (cmd, _) = k.create_subkey("command")?;
        cmd.set_value("", &format!("\"{}\" {args}", exe.display()))?;
    }
    Ok(())
}

pub fn unregister() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for (class, id, ..) in VERBS {
        let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{class}\{id}"));
    }
}

/// Repairs what Explorer hands us before the path is trusted anywhere.
pub fn normalize_shell_arg(raw: &str) -> Option<std::path::PathBuf> {
    let s = raw.trim().trim_end_matches('"');
    let s = if s.len() == 2 && s.ends_with(':') { format!("{s}\\") } else { s.to_owned() };
    let p = std::path::PathBuf::from(s);
    (p.is_absolute() && std::fs::symlink_metadata(&p).is_ok()).then_some(p)
}
```

UIPI fix for forwarding (features `Win32_UI_WindowsAndMessaging`):

```rust
// In .setup(): plugins are set up before this runs. The class name is the plugin's own
// convention, so re-check it when the plugin is upgraded.
let class = windows::core::HSTRING::from(format!("{}-sic", app.config().identifier));
if let Ok(hwnd) = unsafe { FindWindowW(&class, windows::core::PCWSTR::null()) } {
    let _ = unsafe { ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None) };
}
```

Modern handler skeleton (separate `cdylib`, Windows 11 only, needs identity):

```rust
#[windows::core::implement(IExplorerCommand)]
struct Analyze;
impl IExplorerCommand_Impl for Analyze_Impl {
    fn GetTitle(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> { unsafe { SHStrDupW(w!("Analyze with ReClaim")) } }
    fn GetIcon(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> { /* "<installdir>\\reclaim-cleaner.exe,0" */ todo!() }
    fn GetToolTip(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> { Err(E_NOTIMPL.into()) }
    fn GetCanonicalName(&self) -> Result<GUID> { Ok(CLSID_ANALYZE) }
    fn GetState(&self, _: Ref<IShellItemArray>, _slow: BOOL) -> Result<u32> { Ok(ECS_ENABLED.0 as u32) }
    fn Invoke(&self, items: Ref<IShellItemArray>, _: Ref<IBindCtx>) -> Result<()> {
        // items.GetItemAt(0)?.GetDisplayName(SIGDN_FILESYSPATH)? then CreateProcessW(exe --analyze path).
        // Keep it short: this runs in a dllhost surrogate on Explorer's behalf.
        todo!()
    }
    fn GetFlags(&self) -> Result<u32> { Ok(ECF_DEFAULT.0 as u32) }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> { Err(E_NOTIMPL.into()) }
}
```

### Effort and safety

- Classic verbs, toggle, argument handling, debounce, UIPI fix and NSIS/MSI cleanup: **M**.
- Modern handler: **L**, plus the signing certificate and its CI.
- Safety:
  - "Delete safely" must open the app's normal preview and confirm flow (Recycle Bin by default, lock check first). It never deletes directly from the verb.
  - Treat forwarded args as untrusted.
  - The integration is off by default and fully removable.

### APIs and docs

- Creating shortcut menu handlers (static verbs, `MultiSelectModel` limits, `ExtendedSubCommandsKey`, `Extended`, `AppliesTo`, `Position`): https://learn.microsoft.com/en-us/windows/win32/shell/context-menu-handlers
- Predefined shell objects: https://learn.microsoft.com/en-us/windows/win32/shell/handlers
- Windows 11 context menu: https://blogs.windows.com/windowsdeveloper/2021/07/19/extending-the-context-menu-and-share-dialog-in-windows-11/
- Packaging with external location: https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps
- Manifest extensions (`windows.fileExplorerContextMenus`, `com:SurrogateServer`): https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions
- `desktop4:FileExplorerContextMenus`: https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop4-fileexplorercontextmenus
- `desktop5:ItemType`: https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop5-itemtype
- Unsigned packages: https://learn.microsoft.com/en-us/windows/msix/package/unsigned-package
- `IExplorerCommand`: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iexplorercommand
- `ChangeWindowMessageFilterEx`: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-changewindowmessagefilterex
- Tauri installer hooks and WiX fragments: https://v2.tauri.app/distribute/windows-installer/
- Tauri single instance: https://v2.tauri.app/plugin/single-instance/
- 26H2 redesign (secondary): https://www.pcworld.com/article/2964382/windows-11s-decluttered-right-click-menu-draws-criticism-for-one-big-flaw.html and https://windowslatest.com/2026/08/20/will-windows-11-remove-the-windows-10-era-right-click-menu-microsoft-reveals-whats-next/

---

## 2. "Which process is locking this file or folder"

### Approach

1. **Restart Manager (RM)** for files. Create a session (`RmStartSession`), register paths (`RmRegisterResources`), call `RmGetList` (retry on `ERROR_MORE_DATA`), then `RmEndSession`.
   - Each `RM_PROCESS_INFO` gives the PID and **start time** (`RM_UNIQUE_PROCESS`, use it to defeat PID reuse), the app or service display name, the service short name, and `ApplicationType`: `RmUnknownApp`, `RmMainWindow`, `RmOtherWindow`, `RmService`, `RmExplorer`, `RmConsole`, `RmCritical`. It also gives `TSSessionId` and `bRestartable` (verified).
2. **Folders:** RM refuses directories. `RmGetList` returns `ERROR_ACCESS_DENIED` when "a path registered ... is a directory" (verified on Learn, and tested: error 5). For a folder:
   - Register up to N files inside it (for example the first 2,000 by a bounded walk, executables and DLLs first). Batch them in one session.
   - Separately list **processes whose image path is inside the folder** (`QueryFullProcessImageNameW` prefix match). This is the most common reason a folder "is in use".
   - Say clearly that a process using the folder as its **current directory** (for example a terminal opened there) is not detectable without undocumented PEB reads.
3. Show results grouped as: Apps (with a window), Background apps, Services, System (critical). Resolve the image path and publisher for each PID (topic 4).

### What RM sees and cannot see

- Tested non-elevated on this machine:
  - It found a child process holding a temp file (`RmConsole`).
  - For `svchost.exe` it listed 103 entries (96 `RmCritical`, 7 `RmService`, all with service names). So **RM reports processes of other accounts and services without admin**.
  - For `explorer.exe` it reported `RmExplorer`.
- It **cannot** register a file the caller cannot open: `pagefile.sys` gave error 32 (sharing violation) (tested). Registry hives, and other files held exclusively by the kernel or System (PID 4), fall in the same group.
- It cannot see current-directory locks, or handles held by kernel components (SMB server, filter drivers) (unverified for the specific cases, but RM documents only processes and services).
- Limits: 64 RM sessions per user session (verified). Always call `RmEndSession`.
- `RmShutdown` only acts on the whole list of the session. With `RmForceShutdown`, unresponsive apps are force-killed after 30 s and services after 20 s (verified). It also cannot touch other users' or terminal sessions (verified). **Do not use it in the UI.**

### Alternatives considered

- **Handle enumeration** (`NtQuerySystemInformation` with the undocumented class `SystemExtendedHandleInformation`=64, then `DuplicateHandle` plus `NtQueryObject` or `GetFinalPathNameByHandleW`). This is what Process Explorer, LockHunter and Sysinternals `handle` do, mostly with a kernel driver. Reasons not to:
  - The class is undocumented: windows-rs 0.62 defines only the documented classes (verified, it is absent from `Wdk::System::SystemInformation`). Learn warns that `NtQuerySystemInformation` and `NtQueryInformationProcess` "may be altered or unavailable" (verified).
  - `PROCESS_DUP_HANDLE` on other users' and protected processes needs admin and `SeDebugPrivilege`, and is never granted for protected processes (verified, the Learn process rights page).
  - `NtQueryObject` on some pipe handles can hang the calling thread. You need a sacrificial thread with a timeout (well known, unverified here).
  - Windows 11 24H2 zeroes kernel object addresses in these results unless `SeDebugPrivilege` is enabled (secondary, windows-internals.com). This breaks the usual "same file object" matching for medium-integrity callers.
  - Antivirus heuristics dislike tools that duplicate handles out of other processes (unverified).
  - Verdict: **L effort, fragile**. Do not ship. If ever needed, run it only inside the elevated helper behind an "Advanced" switch.
- **Force-closing a handle** in another process (`DuplicateHandle` with `DUPLICATE_CLOSE_SOURCE`) corrupts that process's state. **Never offer it** (this is a product rule, and it matches the task brief).

### Safe actions

- **"Switch to app":** `AllowSetForegroundWindow` + `SetForegroundWindow` on its main window, so the user saves and closes it.
- **"Close app" (graceful):**
  - Post `WM_CLOSE` to the process's visible, unowned top-level windows (`EnumWindows` + `GetWindowThreadProcessId`).
  - Wait a few seconds, then re-run `RmGetList`.
  - Check that the PID's start time still matches before saying "closed".
  - Not for `RmCritical`, `RmService` or `RmExplorer` entries. For Explorer, offer "Restart Explorer" only if you really want it. Better: tell the user to close the Explorer window or preview pane.
  - Elevated target windows: UIPI drops posted messages from medium to high integrity (unverified for `WM_CLOSE` specifically, since UIPI blocks most messages above `WM_USER` and some below). If the target is elevated, say so and suggest closing it by hand.
- **Services:** link to the Services view (topic 3) instead of stopping from here.
- **"Delete on next restart":** `MoveFileExW(path, NULL, MOVEFILE_DELAY_UNTIL_REBOOT)` needs admin (writes `PendingFileRenameOperations`). It could be a helper action for admin-only rules, but it is risky UX. Defer it.

### Rust sketch (`Win32_System_RestartManager`)

```rust
use windows::Win32::System::RestartManager::*;
use windows::Win32::Foundation::{ERROR_MORE_DATA, WIN32_ERROR};
use windows::core::{HSTRING, PCWSTR, PWSTR};

pub fn lockers(files: &[std::path::PathBuf]) -> windows::core::Result<Vec<RM_PROCESS_INFO>> {
    unsafe {
        let mut session = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())).ok()?;
        let res = (|| {
            let wide: Vec<HSTRING> = files.iter().map(|p| HSTRING::from(p.as_os_str())).collect();
            let ptrs: Vec<PCWSTR> = wide.iter().map(|h| PCWSTR(h.as_ptr())).collect();
            RmRegisterResources(session, Some(&ptrs), None, None).ok()?;
            let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
            loop {
                let (mut needed, mut count, mut reasons) = (0u32, infos.len() as u32, 0u32);
                let ptr = (!infos.is_empty()).then(|| infos.as_mut_ptr());
                let r: WIN32_ERROR = RmGetList(session, &mut needed, &mut count, ptr, &mut reasons);
                if r == ERROR_MORE_DATA {
                    infos.resize(needed as usize, std::mem::zeroed());
                    continue;
                }
                r.ok()?;
                infos.truncate(count as usize);
                return Ok(infos);
            }
        })();
        let _ = RmEndSession(session);
        res
    }
}
```

### Effort and safety

- **M**: RM wrapper, folder strategy, image-path match, grouping UI, graceful close with re-check.
- Detection runs as a normal user.
- Never kill or force-close from this screen. Never touch handles.

### APIs and docs

- Restart Manager: https://learn.microsoft.com/en-us/windows/win32/rstmgr/restart-manager-portal
- `RmStartSession`: https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmstartsession
- `RmRegisterResources`: https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmregisterresources
- `RmGetList`: https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmgetlist
- `RM_PROCESS_INFO`: https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/ns-restartmanager-rm_process_info
- `RmShutdown`: https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmshutdown
- `NtQuerySystemInformation`: https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntquerysysteminformation
- Process access rights: https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights
- KASLR restriction in 24H2 (secondary): https://windows-internals.com/kaslr-leaks-restriction/

---

## 3. Services and scheduled tasks

### Services: reading

- `OpenSCManagerW(None, None, SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)`. Local authenticated users have these rights (verified).
- `EnumServicesStatusExW(scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_STATE_ALL, ...)`. Returns name, display name, state, **PID** and `dwServiceFlags` (`SERVICE_RUNS_IN_SYSTEM_PROCESS`).
  - Services the caller lacks `SERVICE_QUERY_STATUS` on are **silently omitted** (verified).
  - Add `SERVICE_DRIVER` only for a read-only "Drivers" tab, if at all.
- Per service: `OpenServiceW(scm, name, SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_ENUMERATE_DEPENDENTS)`. These rights are in the default DACL for local authenticated users (verified).
  - `QueryServiceConfigW`: type, start type, error control, **binary path**, load order group, dependencies (double-NUL list), **account** (`lpServiceStartName`), display name.
  - `QueryServiceConfig2W`:
    - `SERVICE_CONFIG_DESCRIPTION` (may be an indirect `@%SystemRoot%\...dll,-123` string: resolve with `SHLoadIndirectString`).
    - `SERVICE_CONFIG_DELAYED_AUTO_START_INFO`.
    - `SERVICE_CONFIG_TRIGGER_INFO` (trigger-start, shown as "Manual (Trigger Start)").
    - `SERVICE_CONFIG_LAUNCH_PROTECTED` (Windows 8.1+).
    - `SERVICE_CONFIG_SERVICE_SID_INFO`.
    - Max buffer is 8 KB (verified).
  - `EnumDependentServicesW` gives **dependents**. Dependencies come from the config.
- **svchost services:** the binary path is `%SystemRoot%\System32\svchost.exe -k <group> [-p]`. The real code is `HKLM\SYSTEM\CurrentControlSet\Services\<name>\Parameters\ServiceDll` (sometimes directly under the service key). Use that DLL for publisher and signature (well known, unverified wording).
- **Per-user services:**
  - Templates live in `HKLM\SYSTEM\CurrentControlSet\Services` and are hidden from services.msc.
  - Instances are named `<name>_<LUID>`, have type bit `0x40` (`SERVICE_USER_SERVICE`) and instance bit `0x80`, and are created at sign-in and deleted at sign-out.
  - Disabling means setting the **template** `Start=4` (registry) or `UserServiceFlags=0` (verified, Learn per-user services). Tested: 24 of 309 services on this machine have bit 64.
  - Version introduced: Windows 10 1703 or 1709 (unverified).
  - Show instances read-only, and never change templates from the app.
- Parsing:
  - `EnumServicesStatusExW` and `QueryServiceConfig*` fill **byte buffers that contain structs with pointers**. Allocate them as `Vec<u64>` (or an aligned type) and cast, never a plain `Vec<u8>`, to keep alignment.
  - Loop on `ERROR_MORE_DATA` / `ERROR_INSUFFICIENT_BUFFER` with the resume handle.

### Services: classifying protected ones

Compute a `Protection` enum in `reclaim-core` and **recompute it inside the elevated helper**. Never trust a flag from the request.

1. `Driver`: type `SERVICE_KERNEL_DRIVER` or `SERVICE_FILE_SYSTEM_DRIVER`. Read-only.
2. `LaunchProtected`: `SERVICE_CONFIG_LAUNCH_PROTECTED` is not `SERVICE_LAUNCH_PROTECTED_NONE` (antimalware PPL). Read-only. Even admins cannot stop these.
3. `PerUserInstance`: type has `0x80`. Read-only.
4. `HardDeny`: a built-in list of names the app never changes, even if the user insists. Starting set (to be reviewed):
   - `RpcSs`, `RpcEptMapper`, `DcomLaunch`, `LSM`, `SamSs`, `EventLog`, `Winmgmt`
   - `BFE`, `mpssvc`, `WinDefend`, `WdNisSvc`, `SecurityHealthService`, `wscsvc`
   - `wuauserv`, `WaaSMedicSvc`, `UsoSvc`, `TrustedInstaller`, `CryptSvc`
   - `Dnscache`, `nsi`, `Dhcp`, `NlaSvc`, `Power`, `ProfSvc`, `Schedule`, `gpsvc`
   - `BrokerInfrastructure`, `CoreMessagingRegistrar`, `StateRepository`, `AppXSvc`, `ClipSVC`, `sppsvc`, `SystemEventsBroker`, `TimeBrokerSvc`, `UserManager`, `Audiosrv`, `AudioEndpointBuilder`, `PlugPlay`, `DeviceInstall`
5. `Windows`: the image (or `ServiceDll`) is under `%SystemRoot%` **and** carries a valid Microsoft signature (see below). Read-only by default. Optional "expert" switch: allow only Manual or Automatic changes, never Disabled.
6. `Microsoft`: Microsoft-signed but outside `%SystemRoot%` (Office, Edge updaters, OneDrive). Allow stop and start. Start type changes come with a warning.
7. `ThirdParty`: everything else. Allow stop, start, and start type Automatic / Automatic (Delayed) / Manual / Disabled, with a confirmation.

### Signature check (WinVerifyTrust)

- **Most Windows binaries are catalog-signed, not embedded-signed.** Checking with `WTD_CHOICE_FILE` alone reports them as unsigned, so every Windows service would look like third-party. Two steps (verified APIs):
  1. Embedded: `WinVerifyTrust(WINTRUST_ACTION_GENERIC_VERIFY_V2, WTD_CHOICE_FILE)`.
  2. If that fails with `TRUST_E_NOSIGNATURE`:
     - `CryptCATAdminAcquireContext2(..., BCRYPT_SHA256_ALGORITHM)`
     - `CryptCATAdminCalcHashFromFileHandle2` (Windows 8+)
     - `CryptCATAdminEnumCatalogFromHash`
     - `CryptCATCatalogInfoFromContext`
     - `WinVerifyTrust` with `WTD_CHOICE_CATALOG` (member tag = hex hash string)
     - Fall back to SHA-1 for older catalogs (unverified that it is still needed on Windows 10).
- Signer name: `WTHelperProvDataFromStateData` → `WTHelperGetProvSignerFromChain` → `CertGetNameStringW(CERT_NAME_SIMPLE_DISPLAY_TYPE)`. Always end with `WTD_STATEACTION_CLOSE` (verified, required for every VERIFY).
- **No network:** `fdwRevocationChecks = WTD_REVOKE_NONE` and `dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE`. Learn says `WTD_CACHE_ONLY_URL_RETRIEVAL` must be set to prevent network retrieval (verified).
- Publisher rule: a valid chain and a signer of "Microsoft Windows", "Microsoft Windows Publisher" or "Microsoft Corporation" (compare the leaf subject; optionally pin to the Microsoft root). Never trust the `CompanyName` version resource for this; anyone can set it.
- Cost: hashing a large file takes time (verified remark). Cache by `(path, size, mtime)` in the data dir.
- windows-rs: `Win32_Security_WinTrust` and `Win32_Security_Cryptography_Catalog` hold all of these (verified). Learn says `CryptCATAdminCalcHashFromFileHandle2` has no import library, but windows-rs links through `windows-link`, so this is not an issue (unverified at link time).

### Services: changing (elevated helper)

- `OpenServiceW` with `SERVICE_CHANGE_CONFIG` / `SERVICE_START` / `SERVICE_STOP` / `SERVICE_QUERY_STATUS`. Admin is needed for third-party services with the default DACL (verified table).
- Start type: `ChangeServiceConfigW(h, SERVICE_NO_CHANGE, start, SERVICE_NO_CHANGE, None, ...)`, plus `ChangeServiceConfig2W(SERVICE_CONFIG_DELAYED_AUTO_START_INFO)` for Delayed.
- Stop: `ControlServiceExW(SERVICE_CONTROL_STOP, SERVICE_CONTROL_STATUS_REASON_INFO)` with a planned reason, or plain `ControlService`. Then poll `QueryServiceStatusEx` honoring `dwWaitHint`, with a 30 s cap.
  - `ERROR_DEPENDENT_SERVICES_RUNNING`: show the running dependents, and do not stop them automatically.
- Start: `StartServiceW(h, None)`.
- Some Windows services have locked registry ACLs. `ChangeServiceConfigW` returns `ERROR_ACCESS_DENIED` even for admins (for example WaaSMedicSvc) (unverified per service). Map that to "Windows protects this service".
- **Write every change to History with the old value**, and offer one-click revert. Never batch "disable N services".

### Scheduled tasks (Task Scheduler 2.0 COM)

- Run on a worker thread with `CoInitializeEx(None, COINIT_MULTITHREADED)`, not the WebView thread.
- `CoCreateInstance::<_, ITaskService>(&TaskScheduler, None, CLSCTX_INPROC_SERVER)`, then `Connect` with four empty `VARIANT`s, then `GetFolder("\\")`.
- Walk recursively with `GetFolders(0)` and `GetTasks(TASK_ENUM_HIDDEN)` (verified constant, value 1).
- Per `IRegisteredTask`:
  - `Name`, `Path`, `Enabled`, `State`, `LastRunTime`, `NextRunTime`, `LastTaskResult`.
  - `Definition` → `RegistrationInfo.Author` and `Description`, `Principal.UserId` and `RunLevel`, `Triggers`.
  - `Actions`: cast each to `IExecAction` (`Path`, `Arguments`, `WorkingDirectory`) or `IComHandlerAction` (`ClassId`: resolve it through `HKCR\CLSID\{...}\InprocServer32`).
- Tested non-elevated: 202 tasks visible across all folders, with no access-denied folders on this machine. Some tasks may still be filtered silently (unverified).
- Enable or disable: `IRegisteredTask::SetEnabled(VARIANT_BOOL)`. VARIANT_TRUE is -1 (verified).
  - Tasks the user registered can usually be changed without admin. Tasks with a SYSTEM principal or a restrictive security descriptor need the helper. Pattern: try non-elevated, and on `E_ACCESSDENIED` offer elevation (unverified which tasks need what).
- Policy:
  - Tasks under `\Microsoft\` are read-only in the UI.
  - Third-party tasks get enable/disable only, never delete.
  - Show the action exe with its signature, and link to the owning app (topic 5).

### Safe UX

- No presets, no "disable all", no "optimize" button, no "recommended to disable" lists.
- Each change is one service or one task, with a confirmation that says what it does, a link to dependents, and the exact old and new value.
- Default view: hide Windows and Microsoft services behind a "Show Windows services" filter. Group them by publisher.
- Show the reason why something is read-only ("Protected by Windows", "Security software", "Driver", "Per-user service").

### Rust sketch (service list, `Win32_System_Services`)

```rust
use windows::Win32::System::Services::*;

pub unsafe fn list_services() -> windows::core::Result<Vec<(String, String, u32, u32)>> {
    let none = windows::core::PCWSTR::null();
    let scm = OpenSCManagerW(none, none, SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)?;
    let (mut needed, mut count, mut resume) = (0u32, 0u32, 0u32);
    let mut buf: Vec<u64> = Vec::new(); // u64 keeps the struct-with-pointers buffer aligned
    loop {
        let bytes = std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8);
        let r = EnumServicesStatusExW(scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_STATE_ALL,
            Some(bytes), &mut needed, &mut count, Some(&mut resume), windows::core::PCWSTR::null());
        match r {
            Ok(()) => break,
            Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => buf.resize(buf.len() + (needed as usize + 7) / 8, 0),
            Err(e) => { let _ = CloseServiceHandle(scm); return Err(e) }
        }
    }
    let rows = std::slice::from_raw_parts(buf.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(), count as usize);
    let out = rows.iter().map(|s| (
        s.lpServiceName.to_string().unwrap_or_default(),
        s.lpDisplayName.to_string().unwrap_or_default(),
        s.ServiceStatusProcess.dwCurrentState.0,
        s.ServiceStatusProcess.dwProcessId,
    )).collect();
    let _ = CloseServiceHandle(scm);
    Ok(out)
}
```

Note: if the buffer is resized mid-enumeration with a nonzero resume handle, collect the rows from each successful chunk before the next call. The sketch simplifies this.

### Effort

| Part | Effort |
|---|---|
| Service list + config + classification + signature cache | M |
| Service actions in the helper + history and revert | M |
| Tasks read + enable/disable | M |

### APIs and docs

- Service access rights: https://learn.microsoft.com/en-us/windows/win32/services/service-security-and-access-rights
- `EnumServicesStatusExW`: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-enumservicesstatusexw
- `QueryServiceConfigW`: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-queryserviceconfigw
- `QueryServiceConfig2W`: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-queryserviceconfig2w
- `ChangeServiceConfigW`: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-changeserviceconfigw
- `ChangeServiceConfig2W`: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-changeserviceconfig2w
- `ControlServiceExW`: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-controlserviceexw
- `EnumDependentServicesW`: https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-enumdependentservicesw
- Per-user services: https://learn.microsoft.com/en-us/windows/application-management/per-user-services-in-windows
- `WinVerifyTrust`: https://learn.microsoft.com/en-us/windows/win32/api/wintrust/nf-wintrust-winverifytrust
- `WINTRUST_DATA`: https://learn.microsoft.com/en-us/windows/win32/api/wintrust/ns-wintrust-wintrust_data
- `CryptCATAdminCalcHashFromFileHandle2`: https://learn.microsoft.com/en-us/windows/win32/api/mscat/nf-mscat-cryptcatadmincalchashfromfilehandle2
- Task Scheduler (`ITaskService`): https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nn-taskschd-itaskservice
- `ITaskFolder::GetTasks`: https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-itaskfolder-gettasks
- `IRegisteredTask::put_Enabled`: https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-iregisteredtask-put_enabled

---

## 4. Running processes view

### Approach (own thin layer)

**Listing:** `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)` (the feature is already enabled in core). It gives PID, parent PID, exe name and thread count.

**Per PID, `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`**, then:

| Data | API | Notes |
|---|---|---|
| Image path | `QueryFullProcessImageNameW(PROCESS_NAME_WIN32)` | |
| Command line | `NtQueryInformationProcess(ProcessCommandLineInformation = 60)` | Returns a `UNICODE_STRING`. Tested: works with only `PROCESS_QUERY_LIMITED_INFORMATION` for every process that could be opened. Not listed on the Learn page, but defined in the WDK metadata that windows-rs uses (`Wdk::System::Threading`). Load-time risk is low because windows-rs links ntdll dynamically |
| Memory | `GetProcessMemoryInfo` → `PROCESS_MEMORY_COUNTERS_EX` | `WorkingSetSize`, `PrivateUsage`. Documented for `PROCESS_QUERY_LIMITED_INFORMATION` on Windows 8.1+ (unverified wording) |
| CPU % | `GetProcessTimes` sampled twice, divided by wall time × logical CPU count | Or use `GetSystemTimes` deltas for the total |
| Start time | `GetProcessTimes` creation time | |
| Store package | `GetPackageFullName(handle)` | `APPMODEL_ERROR_NO_PACKAGE` for desktop apps. Maps to a Store app |
| Session and user | `ProcessIdToSessionId`, and the token user if `OpenProcessToken` succeeds | Usually same-user only |

**Coverage, tested non-elevated:** 450 of 608 PIDs open with `PROCESS_QUERY_LIMITED_INFORMATION`. The other 156 (System, protected, and services under other accounts) show name and PID only, with "Run as administrator for details".

**Publisher:**
- Fast: `GetFileVersionInfoSizeW` / `GetFileVersionInfoW` / `VerQueryValueW` (`\VarFileInfo\Translation`, then `\StringFileInfo\<lang><cp>\CompanyName`, `FileDescription`, `ProductName`). Not trustworthy.
- Trusted: the signature check from topic 3, computed lazily for visible rows and cached.

**Icons:** `SHGetFileInfoW(SHGFI_ICON | SHGFI_SMALLICON)` or `IShellItemImageFactory`. Convert to PNG once per exe path and cache.

**Open file location:** `tauri-plugin-opener` already implements `reveal_item_in_dir` with `SHOpenFolderAndSelectItems` (verified in the 2.7.2 source). Call it from Rust (`app.opener().reveal_item_in_dir(path)`). The WebView then needs no new permission.

**Process to installed app:**
1. Store: package family name from `GetPackageFullName` matches the Store app list.
2. Desktop: the **longest-prefix match** of the image path against all `InstallLocation` values (normalized: lowercase, trailing backslash, env vars expanded, `\\?\` removed). Then the directories of `DisplayIcon` and `UninstallString`.
3. Services: the PID map from `EnumServicesStatusExW` (`SC_ENUM_PROCESS_INFO`) gives "hosts services X, Y".
4. Unknown: "Not linked to an installed app" (common for portable tools).

**Alternative:** `sysinfo` 0.39.6 depends on `windows` `>=0.62, <0.63` (no duplicate) plus `ntapi`. It provides name, exe, cmd, memory, cpu_usage (two refreshes), start_time and parent.
- Its Windows command line comes from reading the PEB, which needs `PROCESS_VM_READ` and so works for own-user processes only (unverified for 0.39).
- It adds a dependency for things that are about 200 lines with the APIs above.
- Prefer the own layer. Both are fine.

### Safe actions

| Action | Rule |
|---|---|
| Open file location | Always allowed |
| Search the web for the exe name | Opens the default browser through opener |
| Close (graceful) | `WM_CLOSE` to its windows, as in topic 2 |
| End process (`TerminateProcess`) | Only for same-session, same-user, non-critical processes (`IsProcessCritical` is false), never services or protected processes, with a confirmation that names unsaved-data loss. Product decision. Task Manager offers it, but the brief favors graceful |
| Never offered | Suspend, priority or affinity changes, "boost" |

### Rust sketch (command line)

```rust
use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows::Win32::Foundation::{CloseHandle, UNICODE_STRING};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

pub unsafe fn command_line(pid: u32) -> Option<String> {
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut len = 0u32;
    let _ = NtQueryInformationProcess(h, ProcessCommandLineInformation, std::ptr::null_mut(), 0, &mut len);
    let mut buf = vec![0u64; (len as usize + 7) / 8 + 1];
    let st = NtQueryInformationProcess(h, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut len);
    let _ = CloseHandle(h);
    if st.is_err() { return None; }
    let us = &*(buf.as_ptr() as *const UNICODE_STRING);
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2)))
}
```

### Effort, privileges, pitfalls

- **Effort: M.**
- **Privileges:** user for most data. The elevated view needs the app restarted as admin. Do not move live process listing into the one-shot helper.
- **Pitfalls:**
  - PID reuse: key rows by (PID, creation time).
  - `QueryFullProcessImageNameW` returns `\Device\...` paths with `PROCESS_NAME_NATIVE`. Use `PROCESS_NAME_WIN32`.
  - Minimal processes (`Registry`, `Memory Compression`, `System`, `Secure System`) have no image path.
  - WoW64 processes are fine with these APIs.
  - Refresh every 2 s at most, on a background thread. Signature checks only for visible rows.
- **Windows 10 vs 11:** same APIs. "Efficiency mode" (power throttling) is Windows 11 only. Skip it.

### APIs and docs

- `QueryFullProcessImageNameW`: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew
- `NtQueryInformationProcess`: https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntqueryinformationprocess
- `GetProcessMemoryInfo`: https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo
- `GetProcessTimes`: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes
- `VerQueryValueW`: https://learn.microsoft.com/en-us/windows/win32/api/winver/nf-winver-verqueryvaluew
- `GetPackageFullName`: https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getpackagefullname
- `SHOpenFolderAndSelectItems`: https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shopenfolderandselectitems
- Process access rights: https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights

---

## 5. App ownership: everything an app owns

### Input forms

1. An installed app (an `InstalledApp` from `apps.rs`).
2. An exe path. Map it to an app as in topic 4, or treat it as "portable app at this folder".
3. A window ("hunter" crosshair). The window leads to a PID, then the exe, then (2).

### Evidence collectors

Each collector returns `Evidence { path_or_key, kind, source, strength }`. Strength is either **Explicit** (a registration that names the exe or the folder) or **Name** (a name-based guess).

| Collector | Where | Strength | Admin? |
|---|---|---|---|
| Uninstall entry | `HKLM`, `HKLM\...\WOW6432Node`, `HKCU` `...\CurrentVersion\Uninstall\<key>`: `InstallLocation`, `DisplayIcon` (strip `,N`), `UninstallString` and `QuietUninstallString` (dir of the exe, but not `msiexec`), `InstallSource` (evidence only) | Explicit | No |
| MSI | `MsiEnumProductsExW` (current user + machine). `MsiGetProductInfoExW(INSTALLPROPERTY_INSTALLLOCATION)` is often empty. Component paths: `MsiEnumComponentsExW` + `MsiEnumClientsExW`, then `MsiGetComponentPathExW` | Explicit | Other users' per-user installs need admin (verified) |
| MSI fast path | `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\Folders` (folder refcounts written by Windows Installer). Tested readable non-admin, 5,206 values here. BCU uses it (verified in the source). Undocumented key | Explicit | No |
| Store app | `PackageManager::FindPackagesByUserSecurityId("")` (current user, no admin). `Package.InstalledPath`, `Id.FamilyName`. Data: `%LOCALAPPDATA%\Packages\<PFN>`. Aliases: `%LOCALAPPDATA%\Microsoft\WindowsApps\<PFN>` | Explicit | No (all-users `FindPackages()` needs admin, unverified) |
| App Paths | `HKLM`/`HKCU` `\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\<exe>` | Explicit | No |
| Services | `ImagePath` or `ServiceDll` inside the app folder | Explicit | No |
| Scheduled tasks | `IExecAction.Path` inside the app folder | Explicit | No |
| Startup | `startup.rs` sources (Run keys, Startup folders: resolve `.lnk` with `IShellLinkW` or the `lnk` crate) | Explicit | No |
| Firewall rules | `INetFwPolicy2.Rules` → `ApplicationName`. Tested non-admin: 562 rules. Registry alternative: `HKLM\SYSTEM\...\FirewallPolicy\FirewallRules` (readable non-admin, but the string format is undocumented) | Explicit | Read: no. Delete: admin |
| COM and shell extensions | `HKCR\CLSID\{..}\InprocServer32` / `LocalServer32` (and `WOW6432Node`, HKCU) pointing into the folder. Then find referencing `*\shellex\ContextMenuHandlers`, `Directory\shellex`, `Drive\shellex`, `Folder\shellex`, `ShellIconOverlayIdentifiers`, and `PropertySheetHandlers` keys that name that CLSID | Explicit | Read: no |
| File associations | `HKCU`/`HKLM\Software\Classes\<ProgID>\shell\*\command` naming the exe. `Applications\<exe>`. `RegisteredApplications` → `Capabilities` | Explicit | Read: no |
| Shortcuts | Start Menu (`%APPDATA%` and `%PROGRAMDATA%\Microsoft\Windows\Start Menu\Programs`), Desktop. Targets inside the folder | Explicit | No |
| Data folders | `%APPDATA%`, `%LOCALAPPDATA%`, `%LOCALAPPDATA%Low` (`FOLDERID_LocalAppDataLow`), `%PROGRAMDATA%`, `Documents\<name>`. Look for `<Publisher>\<Product>` and `<Product>`, at most 2 levels deep | Name | No |
| Registry data | `HKCU\Software\<Publisher>\<Product>` and `HKLM\SOFTWARE\[WOW6432Node\]<Publisher>\<Product>` | Name | Read: no. Delete HKLM: admin |
| Usage evidence | UserAssist (`HKCU\...\Explorer\UserAssist\{GUID}\Count`, value names ROT13-encoded, undocumented). Tested readable | Evidence only | No |
| Usage evidence | Prefetch `C:\Windows\Prefetch\<EXE>-<hash>.pf`: the file's LastWriteTime approximates the last run. Do not parse it (compressed format since Windows 10, undocumented). Tested: access denied without admin | Evidence only | Yes |
| Usage evidence | BAM `HKLM\SYSTEM\CurrentControlSet\Services\bam\State\UserSettings\<SID>`. Tested: access denied non-admin. Undocumented | Evidence only | Yes |
| Usage evidence | Amcache (`C:\Windows\AppCompat\Programs\Amcache.hve`). The system keeps it loaded and locked, so it needs raw or VSS copies (unverified). **Skip** | n/a | n/a |

### Store app notes

- `C:\Program Files\WindowsApps` root listing is denied, even with admin by default. Tested: `UnauthorizedAccessException` non-admin. Individual package folders are listable (tested).
- Use `PackageManager` instead of listing that folder. The current `apps.rs` shells out to PowerShell `Get-AppxPackage`. Switching to `windows::Management::Deployment::PackageManager` (feature `Management_Deployment` + `ApplicationModel`, collections from `windows-collections` 0.3.2, already in Cargo.lock) removes the PowerShell dependency and is faster (unverified speed gain).
- Never delete inside WindowsApps. Store apps are removed only with `PackageManager.RemovePackageAsync` or the Settings app.

### Confidence scoring

Same idea as BCU (Apache-2.0); our own weights, no code copied.
- **BCU's design**, from reading its sources at a high level:
  - Separate "junk finders": install location, common drive folders, uninstaller location, Prefetch, WER, shortcuts, startup, AppCompatFlags, COM, event log, firewall rules, Installer\Folders, RegisteredApplications, Software keys, uninstaller keys, UserAssist.
  - Each finding carries a list of signed confidence records, for example "explicit connection" (+4), "is the uninstaller registry key" (+20), "company name match" (+4), "product name perfect match" (+2), "directory still used" (−7), "executables are present" (−4), "used by a similarly named app" (−2), "is Store app" (−10).
  - Folder name matching is fuzzy, ignores names of 4 characters or fewer, recurses only 2 levels, and skips known folders.
- **Revo Uninstaller:**
  - Runs the vendor uninstaller first, then scans leftovers in Safe, Moderate or Advanced modes (deeper means more false positives).
  - "Hunter mode" lets you drag a target onto a window or shortcut to uninstall, stop autorun, or kill.
  - Its "traced programs" depend on install-time monitoring (secondary; Revo's own docs were not reached).
- **Proposed scoring:**
  - **Hard blocks (never offered, whatever the score):**
    - Any path inside or above another installed app's `InstallLocation`.
    - Known shared folders: `Common Files`, `Microsoft`, `Windows`, `WindowsApps`, `Packages` root, `Temp`, user profile roots, `Program Files` roots, `ProgramData` root, `Microsoft\Windows\Start Menu` root.
    - Anything a running process currently uses (topic 2).
    - System32 / SysWOW64.
    - Registry keys under `HKLM\SYSTEM`, except the service entry of a third-party service the app owns, handled via SCM.
  - **Explicit evidence:** "High". Preselect only these, and only if the app is already uninstalled (leftover mode).
  - **Name evidence:** score = exact product-name folder (+3) + under a publisher folder that matches (+3) + publisher-only folder with no other product subfolders (+1).
    - Penalties: contains executables whose signer differs from the app's signer (−4); a fuzzy name only (−3); name of 4 characters or fewer (no name scoring at all); used by a similarly named installed app (−4); recently modified while the app is "uninstalled" (−2).
    - Score ≥ 5 is "Medium", shown unselected. Lower is "Low", hidden behind "Show uncertain items".
  - **Normalization:** lowercase; strip version numbers, architecture tags ("x64", "(64-bit)") and legal suffixes ("Inc.", "Ltd", "GmbH", "LLC", "Corporation"); collapse spaces, underscores and dashes.
  - **Every item shows *why*** (the evidence list), like BCU's confidence records.
- **Action path:**
  - Folders go through the existing preview → Recycle Bin pipeline.
  - Registry keys: export to `.reg` in the History folder before deleting (`RegSaveKeyEx` needs `SeBackupPrivilege`; a manual export is simpler). Deleting HKLM keys is a helper operation with ids that map to items the helper recomputes, never raw key paths.

### Window under the cursor ("hunter")

- A WebView cannot capture the mouse outside the window. Do it in Rust:
  - **Option A (simple, AV-friendly):** the user clicks "Pick a window". Rust starts a thread that polls `GetCursorPos` + `GetAsyncKeyState(VK_LBUTTON)` every ~30 ms, optionally moving a small topmost, click-through, layered crosshair window. On mouse-up, resolve the window. Esc (polled the same way) cancels.
  - **Option B:** a `WH_MOUSE_LL` hook on a thread with a message loop. It is more precise, but low-level hooks have a system timeout and attract AV heuristics (unverified).
- Resolve:
  - `WindowFromPoint(pt)` → `GetAncestor(hwnd, GA_ROOT)` → `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW`.
  - Ignore our own PID. If the user releases over the ReClaim window, cancel.
  - **UWP / Store apps:** the top-level window belongs to `ApplicationFrameHost.exe`. Enumerate its children (`EnumChildWindows`), find class `Windows.UI.Core.CoreWindow` with a different PID, and use that PID. If the app is suspended, fall back to the frame's AUMID via `SHGetPropertyStoreForWindow` + `PKEY_AppUserModel_ID` (unverified for suspended apps).
  - **Elevated targets:** `WindowFromPoint` and `GetWindowThreadProcessId` work across integrity levels. `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` on an elevated process of the same user normally succeeds (unverified). If it fails, show "Run ReClaim as administrator to inspect this window".
  - DPI: Tauri windows are per-monitor DPI aware, so `GetCursorPos` and `WindowFromPoint` use physical coordinates consistently (unverified edge cases with mixed-DPI monitors).

### Rust sketch

```rust
use windows::Win32::Foundation::{CloseHandle, POINT};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetCursorPos, GetWindowThreadProcessId,
    WindowFromPoint, GA_ROOT};

pub unsafe fn exe_under_cursor() -> Option<(u32, std::path::PathBuf)> {
    let mut pt = POINT::default();
    GetCursorPos(&mut pt).ok()?;
    let top = GetAncestor(WindowFromPoint(pt), GA_ROOT);
    let mut pid = 0u32;
    GetWindowThreadProcessId(top, Some(&mut pid));
    if pid == 0 || pid == std::process::id() { return None; }
    // ApplicationFrameHost.exe: look for the child CoreWindow and use its PID instead.
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
    let _ = CloseHandle(h);
    ok.ok()?;
    Some((pid, String::from_utf16_lossy(&buf[..len as usize]).into()))
}
```

### Effort and safety

| Part | Effort |
|---|---|
| Collectors | Each is S. Together M |
| Scoring + UI with an evidence list | M |
| Hunter | S to M |
| Whole feature | L |

- Never remove anything only because the name matches.
- Never delete inside WindowsApps or `C:\Windows\Installer`.
- Firewall rule and service removal go through the helper, one item at a time, with history.

### APIs and docs

- Uninstall registry key: https://learn.microsoft.com/en-us/windows/win32/msi/uninstall-registry-key
- `MsiEnumProductsExW`: https://learn.microsoft.com/en-us/windows/win32/api/msi/nf-msi-msienumproductsexw
- `MsiEnumComponentsExW`: https://learn.microsoft.com/en-us/windows/win32/api/msi/nf-msi-msienumcomponentsexw
- `MsiEnumClientsExW`: https://learn.microsoft.com/en-us/windows/win32/api/msi/nf-msi-msienumclientsexw
- `MsiGetComponentPathExW`: https://learn.microsoft.com/en-us/windows/win32/api/msi/nf-msi-msigetcomponentpathexw
- `MsiGetProductInfoExW`: https://learn.microsoft.com/en-us/windows/win32/api/msi/nf-msi-msigetproductinfoexw
- `PackageManager`: https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.packagemanager
- `Package.InstalledPath`: https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.package.installedpath
- App registration (App Paths): https://learn.microsoft.com/en-us/windows/win32/shell/app-registration
- `INetFwPolicy2`: https://learn.microsoft.com/en-us/windows/win32/api/netfw/nn-netfw-inetfwpolicy2
- `WindowFromPoint`: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-windowfrompoint
- `GetAncestor`: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getancestor
- `GetWindowThreadProcessId`: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid
- BCU sources (Apache-2.0, read for concepts only): https://github.com/Klocman/Bulk-Crap-Uninstaller/tree/master/source/UninstallTools/Junk
- Revo overview (secondary): https://askleo.com/revo-uninstaller/

---

## 6. Project finder

### Approach

Extend the existing `artifacts::scan_projects` (marker files, artifact rules, `has_git`, `last_activity`) with two extra root sources and richer per-project facts.

### Root sources

**A. IDE recent lists** (fast, accurate, finds projects outside the scan roots):

| IDE | Where | Status |
|---|---|---|
| VS Code (Code, Code - Insiders, VSCodium, Cursor, Windsurf: same layout, different folder name) | **`%APPDATA%\<Code>\User\workspaceStorage\<hash>\workspace.json`** with `{"folder": "file:///c%3A/..."}` or `{"workspace": ".../x.code-workspace"}`. One per folder ever opened, and robust | Tested: 22 entries, all `folder` with `file:` URIs |
| VS Code | `%APPDATA%\Code\User\globalStorage\state.vscdb` (SQLite), table `ItemTable`, key `history.recentlyOpenedPathsList` (JSON `entries[]` with `folderUri` / `fileUri` / `workspace`) | Secondary sources. **Tested: that key was absent on this machine's current VS Code** (the DB has only an `ItemTable` and a `cursorDiskKV` table). Treat it as optional. Open a **temp copy** read-only: VS Code keeps the DB open, and WAL mode can lock it |
| VS Code | `storage.json` in the same folder | Tested: keys are `backupWorkspaces`, `windowsState`, `profileAssociations` and others. `windowsState` / `backupWorkspaces` give currently open folders. No recent list here |
| VS Code URIs | Decode percent-encoding. Skip non-`file` schemes (`vscode-remote://`, WSL) or show them as "remote" without size | |
| JetBrains | `%APPDATA%\JetBrains\<Product><Version>\options\recentProjects.xml` (`RecentProjectsManager`, paths use `$USER_HOME$`) | Config root verified, JetBrains docs. File name and format secondary. Rider is reportedly `recentSolutions.xml` (unverified). Use the newest version folder per product |
| Visual Studio 2017+ | `%LOCALAPPDATA%\Microsoft\VisualStudio\<ver>_<instanceId>\ApplicationPrivateSettings.xml`, setting `CodeContainers.Offline` (JSON with `LocalProperties.FullPath`) | Partly verified: the `CodeContainers.*` setting is registered with `IsStoredOnDisk` in that instance's private hive (`privateregistry.bin`, readable with `RegLoadAppKeyW` on a copy). On this machine the XML file did not exist yet. Exact JSON shape unverified |
| Visual Studio ≤ 2015 | `HKCU\Software\Microsoft\VisualStudio\<ver>\ProjectMRUList` (unverified) | |
| GitHub Desktop | Stores its repo list in IndexedDB/LevelDB. Skip | |

**B. Disk walk** (existing): plus **git repos** = a directory containing `.git` (a folder, or a **file** `gitdir: ...` for worktrees and submodules).
- Do not descend into `node_modules`, `target`, `.venv`, `vendor`, `Pods`, or into other repos' `.git`.
- Treat nested repos (submodules) as children of the parent project.

### Per-project facts

| Fact | Source |
|---|---|
| Size and artifacts | Existing `measure_dir` + artifact rules. Show "artifacts" (rebuildable) vs "source" bytes |
| Last activity | The existing `last_activity` (newest mtime outside artifact folders) |
| Last local git activity, no dependency | **Last line of `.git/logs/HEAD`** (reflog). Format `<old> <new> <name> <email> <unix-ts> <tz>\t<msg>`, documented in git's repository layout. Gives the time of the last commit, checkout, pull or reset. Cheap: read the last 4 KB |
| Remote URL, no dependency | Parse `.git/config` `[remote "origin"] url = ...`. Strip credentials (`https://user:token@host/...` → `https://host/...`) before showing or storing |
| Current branch | `.git/HEAD` (`ref: refs/heads/<name>`) |
| Exact last commit date | `git -C <path> log -1 --format=%ct` (with `CREATE_NO_WINDOW`, a timeout, and `GIT_OPTIONAL_LOCKS=0` so it never writes `index.lock`), **only if `git` is on PATH**. Or `gix` 0.89.0: `gix::open(path)?.head_commit()?.time()` (API names unverified for 0.89). gix is a large dependency tree; the reflog is enough for the list view |
| Dirty or unpushed | `git status --porcelain=v1 -b` (CLI) or gix status. Valuable for the safety warning "uncommitted changes": lazy, on project open only |
| Package manager | Marker files (`package.json` + lockfile type, `Cargo.toml`, `pyproject.toml`, `*.csproj`, `go.mod`, `pom.xml`, `build.gradle`) |

### Safety

- Artifact cleaning stays per folder with preview and Recycle Bin. Never offer "delete project".
- Warn when a project has uncommitted changes, even though artifacts are rebuildable (the user may have unignored files in `build/`).
- Do not follow junctions or symlinks out of the project (the existing `is_link` guard).
- `node_modules` inside `AppData` or Program Files belongs to apps (the existing `SKIP_DIRS`).
- Reading IDE files: read-only, on a temp copy for SQLite. Never write to IDE state.

### Rust sketch (reflog + remote)

```rust
pub fn git_facts(repo: &std::path::Path) -> (Option<i64>, Option<String>) {
    let git = repo.join(".git");
    let gitdir = if git.is_file() {
        std::fs::read_to_string(&git).ok()
            .and_then(|s| s.strip_prefix("gitdir:").map(|p| repo.join(p.trim())))
    } else {
        Some(git)
    };
    let Some(gitdir) = gitdir else { return (None, None) };
    let last = std::fs::read_to_string(gitdir.join("logs").join("HEAD")).ok()
        .and_then(|s| s.lines().last().map(str::to_owned))
        .and_then(|l| l.split('\t').next()?.split(' ').rev().nth(1)?.parse().ok());
    let remote = std::fs::read_to_string(gitdir.join("config")).ok().and_then(|cfg| {
        let mut in_origin = false;
        cfg.lines().map(str::trim).find_map(|l| {
            if l.starts_with('[') { in_origin = l == r#"[remote "origin"]"#; return None; }
            if in_origin { l.strip_prefix("url").map(|r| r.trim_start_matches([' ', '=']).trim().to_owned()) } else { None }
        })
    });
    (last, remote.map(redact_credentials))
}
fn redact_credentials(url: String) -> String {
    match (url.find("://"), url.find('@')) {
        (Some(s), Some(a)) if a > s => format!("{}{}", &url[..s + 3], &url[a + 1..]),
        _ => url,
    }
}
```

Note: worktree `gitdir` points at `.git/worktrees/<name>`, which has its own `HEAD` and `logs/HEAD`, but `config` lives in the common dir (read `commondir`). Handle that or fall back to the CLI.

### Effort

- IDE readers: S each.
- Git facts: S.
- `rusqlite` (0.40.2 with `bundled`, which adds a C build) only if the VS Code SQLite list is wanted. workspaceStorage alone needs no new dependency.
- UI: M.
- Overall **M**.

### Docs

- JetBrains directories: https://www.jetbrains.com/help/idea/directories-used-by-the-ide-to-store-settings-caches-plugins-and-logs.html
- git repository layout (reflog): https://git-scm.com/docs/gitrepository-layout
- gix: https://docs.rs/gix
- `RegLoadAppKeyW`: https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regloadappkeyw

---

## 7. Tauri 2 and helper plumbing

### Tauri

- **No new plugins needed.** Everything is Rust commands. `single-instance`, `opener` and `dialog` are already present.
- **Capabilities:**
  - App commands registered with `generate_handler!` are allowed for the window when no command-level app manifest is declared (`build.rs` uses plain `tauri_build::build()`) (unverified wording; matches current behavior).
  - Keep the WebView without fs access, as today. Call `opener` reveal from Rust so `opener:allow-reveal-item-in-dir` is not needed. If the frontend calls it directly, add that permission with a path scope.
- **COM threads:** Task Scheduler, firewall, `IShellLink`, `PackageManager` (WinRT) and `IShellItemImageFactory` run on `tauri::async_runtime::spawn_blocking` threads that call `CoInitializeEx(COINIT_MULTITHREADED)` (or STA for shell image APIs, unverified which ones need STA). Never on the main or WebView thread.
- **Long scans:** reuse `Job` (cancel and progress) for lock scans over folders, the project finder and app-ownership collectors.
- **Shell args:** one `handle_shell_args(argv, cwd)`, used by `main.rs` (cold start) and the single-instance callback. Emit an event to the frontend (`shell-request`) with a validated, typed payload.

### What needs the elevated helper

| Operation | Helper? | Notes |
|---|---|---|
| Register or unregister Explorer verbs (HKCU) | No | Per user |
| Lock detection (RM) | No | Works for other users' processes (tested) |
| Graceful close (WM_CLOSE) | No | Elevated targets may ignore it (UIPI) |
| Service list, config, signature | No | |
| Service start, stop, start type | **Yes** | Recompute protection in the helper |
| Task list | No | |
| Task enable/disable (SYSTEM or other principal) | **Yes**, on access denied | |
| Process list (full coverage) | Restart the app as admin | Not the one-shot helper |
| App ownership: Prefetch evidence, other users' MSI data | Optional, read-only | |
| Delete HKLM keys or firewall rules, uninstall services of a removed app | **Yes** | Ids only, recomputed |
| Machine-wide sparse package provisioning | Yes | Out of scope |

### Helper protocol

The current request is `{ ids, method, exclusions }` for cleaning. Generalize it to a tagged enum and keep the same file handshake (`data_dir()\elevated\request-<stamp>.json`, result via `create_new` in a real folder):

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum HelperOp {
    Clean { ids: Vec<String>, method: DeleteMethod, exclusions: Vec<String> },
    ServiceSetStart { name: String, start: StartKind },   // StartKind: Auto | AutoDelayed | Manual | Disabled
    ServiceControl { name: String, action: ServiceAction }, // Start | Stop
    TaskSetEnabled { path: String, enabled: bool },
    RemoveOwnedItems { app_id: String, item_ids: Vec<String> }, // ids from a fresh helper-side scan
}
```

Helper rules:
- **Re-derive everything.**
  - Look up the service by exact name through SCM, and recompute `Protection` (including the signature check). Refuse anything but `ThirdParty` / `Microsoft` (policy).
  - Check that a task path exists and is not under `\Microsoft\`.
  - For owned items, the helper re-runs the collectors for `app_id` and deletes only items whose recomputed ids match.
- One operation per request (or a short list), each with its own result entry. The result carries the before and after values for History.
- **Threat model:** the request file sits in a user-writable folder, so another medium-integrity process could edit it between write and the elevated read. Ids and enums limit what it can do. The UAC prompt is the real boundary (malware at medium integrity can trigger UAC anyway). Keep it that way. Never put paths, command lines or registry key paths in requests.

---

## Windows 10 vs 11 differences that matter here

| Area | Windows 10 | Windows 11 |
|---|---|---|
| Classic registry verbs | Top-level menu | Under "Show more options" / Shift+F10. 26H2 (Insider, secondary): "Show more options" may be off by default and need a Settings switch |
| IExplorerCommand + identity | Supported from 2004 (19041) for external-location packages. desktop5 ItemType from 1809 | Required for the top-level menu |
| Unsigned MSIX | Not available | Available, needs admin, not for distribution |
| Handle-table kernel addresses | Returned to medium-integrity callers | Zeroed from 24H2 unless `SeDebugPrivilege` (secondary) |
| Per-user services | Present (from about 1703/1709, unverified) | Present |
| `ProcessCommandLineInformation` | Available (Windows 8.1+) | Available |
| `CryptCATAdminCalcHashFromFileHandle2` | Available (Windows 8+) | Available |

---

## Local test log (Windows 11 Pro 26200, not elevated)

| Test | Result |
|---|---|
| RM on a temp file held by a child process | Found, `RmConsole` |
| RM on a directory | Error 5 (`ERROR_ACCESS_DENIED`), matches Learn |
| RM on `C:\pagefile.sys` | Error 32 (sharing violation): RM cannot register it |
| RM on `svchost.exe` image | 103 entries (96 critical, 7 services), all with service names |
| RM on `explorer.exe` | 1 entry, `RmExplorer` |
| `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` over all PIDs | 450 / 608 succeed. Image path fails for 2 of those |
| `NtQueryInformationProcess(60)` on those handles | 450 / 450 succeed |
| Firewall `INetFwPolicy2.Rules` | 562 rules readable |
| Task Scheduler walk with hidden tasks | 202 tasks, 0 denied folders |
| `Installer\Folders` key | Readable, 5,206 values |
| Firewall rules registry key | Readable, 562 values |
| `C:\Windows\Prefetch` | Access denied |
| BAM key | Access denied |
| UserAssist | Readable |
| `C:\Program Files\WindowsApps` root | Access denied. A package folder inside is listable |
| Per-user services | 24 of 309 have type bit 64 |
| VS Code `workspaceStorage` | 22 `workspace.json` files, all `{"folder": "file:///..."}` |
| VS Code `state.vscdb` | Tables `ItemTable` and `cursorDiskKV`. No `history.recentlyOpenedPathsList` key |
| Visual Studio 17 private hive | `RegLoadAppKeyW` on a copy works. `CodeContainers.*` setting registered with `IsStoredOnDisk` |
| tauri-plugin-single-instance 2.5.4 source | `WM_COPYDATA` via `SendMessageTimeoutW`, no message filter change |
| Tauri `installer.nsi` | Uninstall runs with `$UpdateMode` during updates |
| Tauri `main.wxs` | `InstallScope="perMachine"` |

---

## Top risks

1. **Windows 11 context menu visibility.** Registry verbs are already hidden one level down on Windows 11. They may need a Settings switch on 26H2 if the Insider design ships. The real fix needs code signing plus a COM DLL in an Explorer surrogate.
2. **Lost shell requests when ReClaim runs elevated.** The single-instance plugin uses `WM_COPYDATA`, which UIPI blocks. Fixing it widens who can send args, so the args must be treated as untrusted.
3. **Service changes can break Windows.** Classification must be computed in the helper, catalog signatures must be checked correctly, and there must be no bulk actions, presets or "recommended" lists.
4. **App ownership false positives.** Shared vendor folders, similar product names, apps that share runtimes. Only explicit evidence may be preselected, and every item shows its evidence.
5. **Undocumented sources** (handle tables, Prefetch format, BAM, Amcache, `Installer\Folders`, firewall registry strings, VS Code and Visual Studio state formats) can change between builds. Use them as optional evidence with graceful failure, never as the basis for deletion.
6. **Drive-root argument quoting** (`"C:\"` → `C:"`) and virtual-folder targets from Explorer.

## Sources not reached

The web search budget for this session ran out before these could be checked:
- Code-signing eligibility (Azure Artifact Signing for individuals outside the US or Canada, SignPath OSS terms).
- The official Windows Insider post text for the 26H2 menu.
- The exact JSON shape of Visual Studio `CodeContainers.Offline`.
- Rider's `recentSolutions.xml` format.
