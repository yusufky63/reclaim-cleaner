# Research 2: Feature gaps for ReClaim Cleaner

Date: 2026-10-16. Baseline: ReClaim 0.1.0 (see `CHANGELOG.md`, `ROADMAP.md`, `docs/research.md`).
Goal: find what users want from a Windows disk cleaner, space analyzer and light system utility that ReClaim does not do yet, and decide what fits its "safe, honest, no tweaks" stance.

## 0. Method and limitations

- Sources: vendor docs and changelogs, GitHub issue trackers (WinDirStat, Czkawka, BCUninstaller, BleachBit, PowerToys, WSL), Microsoft Q&A, ElevenForum, TenForums, AskWoody, Steam Community, Docker forum, tech press.
- **Reddit could not be read.** reddit.com is blocked for both the web search tool and the browser pane, and the Reddit archive API rate-limited every request. The Reddit links below come from search-engine result titles and snippets only; the thread bodies were not read. Treat them as "this question exists and is common", not as verified content.
- The shared web-search budget ran out near the end. Items checked only from memory are marked **[unverified]**.
- Some vendor facts come from third-party listings (Neowin, ghacks, download sites) instead of the vendor's own pages. Those are marked "secondary".
- No text is copied from GPL tools or rule files. Everything here is a feature idea, summarized in my own words.

## 1. Summary

1. **"C: is full and the folders do not add up" is the most common question by far**, and no tool answers it well. Space analyzers can only show files they can read. The gap usually comes from System Restore/shadow copies, reserved storage, the page file, hibernation, NTFS metadata, folders the scanner could not open, and WSL/Docker disks. A page that reconciles *used space* against *scanned space* and names each hidden part would set ReClaim apart (DaisyDisk does this on macOS; WizTree only explains the gap in its release notes).
2. **"What is X and can I delete it?"** (WinSxS, `Windows\Installer`, AppData, Package Cache, System Volume Information, CBS logs, shader caches). ReClaim already covers many of these with honest rule texts. The gaps are `Windows\Installer` orphans, restore points, the search index, old user profiles, iPhone backups and the DriverStore.
3. **WSL and Docker virtual disks never shrink.** The WSL issue for this has been open since 2019 with hundreds of comments, and there are many one-off GitHub scripts. ReClaim measures these disks but cannot compact them yet. WinDirStat 2.5 added "optimize VHD".
4. **Table-stakes analyzer features are still missing:** an Explorer "Analyze with ReClaim" menu entry, saved scans and comparisons, filter syntax, export, and the MFT fast scan. The scan comparison, filter syntax and MFT scan are already on the roadmap.
5. **Uninstall quality is where users feel the difference** (leftovers, batch uninstall, apps without a working uninstaller). BCU's issue tracker shows both kinds of failure: leftovers that are missed, and shared folders that get deleted when they should not.
6. **"Which program is locking this file?"** has its own tools (LockHunter, PowerToys File Locksmith). A read-only Restart Manager lookup fits ReClaim well.
7. **What not to copy:** registry cleaning, RAM and performance "boosters", driver and software updaters, secure wipe and trace scrubbing, and generic junction-based "move this program to D:".

---

## 2. Feature matrix

Legend: **Y** = has it, **P** = partial / paid tier only / limited, **-** = no, **?** = could not verify, **n/a** = does not apply to that platform.

### 2.1 Space analysis

| Feature | ReClaim 0.1 | WizTree | TreeSize Free | TreeSize Pro | SpaceSniffer | WinDirStat 2.x | Everything | DaisyDisk (mac) | CleanMyMac Space Lens (mac) | Mole (mac CLI) |
|---|---|---|---|---|---|---|---|---|---|---|
| Fast NTFS MFT scan | - (roadmap) | Y | ? | ? | - | Y (2.5) | Y (index) | n/a | n/a | n/a |
| Treemap | Y (3 levels) | Y | ? | Y | Y | Y | - | - (sunburst) | P (bubbles) | - |
| Other visualizations (sunburst, flame) | - | - | - | Y (charts) | - | Y (2.8) | - | Y | Y | - |
| File type statistics | Y | Y | ? | Y | - | Y | P | - | - | - |
| Filter / search syntax | - (roadmap) | Y (wildcards) | ? | Y | Y | Y (2.5-2.9) | Y | - | - | P |
| File age filter or view | - (roadmap) | ? | ? | Y [unverified] | Y | Y (2.6) | Y | - | ? | - |
| Allocated vs logical size | Y | Y | Y | Y | ? | Y | ? | ? | ? | ? |
| Hard links counted once | Y | Y | ? | ? | ? | Y (2.5) | n/a | ? | ? | ? |
| Cloud-only (placeholder) awareness | P (never hydrates) | ? | ? | ? | ? | Y (physical size, 2.5) | ? | n/a | Y (cloud) [unverified] | n/a |
| Save and reload a scan | - | Y (CSV/MFT import) | - | Y (XML) | Y (snapshot) | Y (JSON) | n/a | - | - | - |
| Compare scans / growth over time | - (roadmap) | P (re-import) | - | Y | - | - | - | - | - | - |
| Scheduled scans | - | - | - | Y | - | - | - | - | - | - |
| Export (CSV / JSON / report) | P (PowerShell plan) | Y (CSV, image) | ? | Y | Y (text) | Y (CSV, JSON) | Y | - | - | Y (`--json`) |
| Command line | - (roadmap) | Y | ? | Y | - | Y | Y (es.exe) [unverified] | - | - | Y |
| Explorer context menu | - | Y | Y | Y | ? | Y | ? | n/a | n/a | n/a |
| Owner column | - | ? | ? | Y [unverified] | - | Y | - | - | - | - |
| Duplicate finder | Y | - | - | Y | - | Y | - | - | Y [unverified] | - |
| Hard-link deduplication | - (roadmap P2) | - | - | Y | - | ? | - | - | - | - |
| NTFS compression action | - (roadmap P2) | - | - | ? | - | Y | - | n/a | n/a | n/a |
| Empty folder tool | Y | - | - | ? | - | Y (2.7) | - | - | - | - |
| Live updates while open | - | Y | ? | ? | Y | Y (file watcher) | Y | - | - | - |
| Hidden / unexplained space accounting | - | P (explained in notes) | - | ? | - | - | - | Y (hidden, purgeable) | P (purgeable) | - |
| Staging basket before delete | - | - | - | - | - | - | - | Y (collector) | P | - |
| Move files from the analyzer | - | - | - | Y | - | Y (2.5) | - | - | - | - |
| Deleted-file recovery | - | - | - | - | - | Y (2.9) | - | - | - | - |
| Shortcuts to system tools (DISM, VHD, Programs) | Y | - | - | - | - | Y | - | - | - | - |

### 2.2 Cleaning suites and cleaners

| Feature | ReClaim 0.1 | CCleaner | BleachBit | PrivaZer | Wise Disk Cleaner / Care 365 | Glary Utilities | MS PC Manager | Storage Sense | Czkawka / Krokiet | CleanMyMac | Mole |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Rule-based cache/junk cleaning | Y (118 rules) | Y | Y | Y | Y | Y | Y | P (temp) | P (basic temp) | Y | Y |
| Running-app check | Y | Y [unverified] | Y | ? | ? | ? | ? | n/a | n/a | ? | ? |
| Preview / dry run | Y | Y | Y | Y | Y | Y | P | - | Y | Y | Y |
| User exclusions / allowlist | Y | Y | Y | ? | ? | ? | ? | - | Y | ? | Y |
| Privacy traces (cookies, history, MRU) | - (by design) | Y | Y | Y | Y | Y | - | - | - | Y | - |
| Registry cleaner | - (by design) | Y | - | Y | Y | Y | - | - | - | n/a | - |
| Shredder / free-space wipe | - (by design) | Y [unverified] | Y | Y | ? | Y | - | - | - | Y [unverified] | - |
| Scheduled / automatic cleaning | - (by design) | Y [unverified] | - | Y [unverified] | Y [unverified] | ? | Y | Y | - | Y [unverified] | - |
| Large files | Y | P (disk analyzer) | - | - | ? | ? | Y | P (recommendations) | Y | Y | Y |
| Duplicates | Y | Y [unverified] | - | - | ? | Y | Y | - | Y | Y [unverified] | - |
| Similar images / videos / music | - | - | - | - | - | - | - | - | Y | Y [unverified] | - |
| Empty folders | Y | - | - | - | - | Y | - | - | Y | - | - |
| Developer build artifacts | Y | - | - | - | - | - | - | - | - | - | Y (`purge`) |
| Package-manager cleanup commands | P (folder rules) | - | P (VS Code, Cursor cleaners) | - | - | - | - | - | - | - | Y |
| Old installer finder | - (roadmap) | - | - | - | - | - | Y (Downloads) | P (Downloads by age) | - | ? | Y (`installer`) |
| Windows system tools (DISM, Windows.old, DO) | Y | P [unverified] | - | - | ? | ? | Y | Y | - | n/a | n/a |
| Uninstaller | Y | Y | - | - | - | Y | P | - | - | Y | Y |
| Startup manager | Y | Y | - | - | Y | Y | Y | - | - | Y | n/a |
| Driver / software updater | - | Y | - | - | ? | ? | - | - | - | Y (apps) | - |
| RAM / performance booster | - | Y (Performance Optimizer) | - | - | Y | Y | Y (Boost) | - | - | Y | P (`optimize`) |
| Health score / one-click checkup | - | Y | - | - | Y | Y | Y | - | - | Y | Y (`status`) |
| Operation history log | Y | ? | P | ? | ? | ? | ? | - | - | ? | Y |

### 2.3 Uninstallers and system utilities

| Feature | ReClaim 0.1 | Revo Uninstaller | Geek Uninstaller | BCUninstaller | Autoruns | Process Explorer / System Informer | LockHunter / IObit Unlocker / File Locksmith | MS PC Manager |
|---|---|---|---|---|---|---|---|---|
| Desktop + Store apps list | Y | Y | Y | Y (+ Steam, Chocolatey, Windows Features, Updates) | - | - | - | P |
| Run vendor uninstaller | Y | Y | Y | Y | - | - | - | P |
| Batch / quiet uninstall | - | P [unverified] | - | Y | - | - | - | - |
| Forced uninstall (broken or missing uninstaller) | - | Y | Y | Y | - | - | - | - |
| Leftover file scan | P (install folder only) | Y | Y | Y | - | - | - | - |
| Leftover registry scan | - | Y | Y | Y | - | - | - | - |
| Confidence levels on leftovers | - (roadmap) | P (tree, user review) | - | Y | - | - | - | - |
| Install monitoring / logs database | - | Y (Pro) | - | - | - | - | - | - |
| Backups of removed items | - | Y | - | ? | - | - | - | - |
| Uninstall by window / shortcut / folder | - | Y (Hunter mode) | - | Y | - | - | - | - |
| Export installed apps list | - | ? | Y (HTML) | Y | - | - | - | - |
| Uninstaller certificate check | - | - | - | Y | - | - | - | - |
| Startup Run keys and folders | Y | Y | - | Y | Y | - | - | Y |
| Scheduled tasks, services, shell extensions, drivers | - | - | - | ? | Y | P (services) | - | - |
| Signature verification / VirusTotal | - | - | - | P | Y | Y | - | - |
| Which process locks a file | - (roadmap) | - | - | - | - | Y (find handle/DLL) | Y | - |
| Kill process / unlock handle | - (by design) | - | - | - | - | Y | Y | Y (process tab) |
| Delete on reboot | - | - | - | - | - | - | Y (LockHunter) | - |
| Restore point before changes | - (roadmap) | Y | - | ? | - | - | - | - |
| Browser extension removal | - | Y (Pro) | - | - | P (listed) | - | - | - |

Sources for the matrix: WinDirStat CHANGELOG; WizTree release notes (Neowin) and analysis guides; TreeSize feature page and manual; SpaceSniffer Chocolatey page; Everything index docs; Mole README; BCU feature page; Geek Uninstaller site and ghacks; Revo (ghacks, WindowsReport, forum posts, secondary); Autoruns Microsoft Learn page; File Locksmith Microsoft Learn page; LockHunter listings (secondary); PrivaZer user guide; Glary Neowin listings (secondary); PC Manager official page plus Windows Latest and hands-on reviews (secondary); DaisyDisk guide. Full links are in the Sources section.

---

## 3. Top user pain points

Each item: what users ask, why current tools leave them stuck, and representative threads.

### P1. "C: is full for no reason / the folders don't add up"
- Users see tens or hundreds of GB "missing". A scanner shows less than Windows reports as used, or space disappears overnight. The usual causes are System Restore and shadow copies (System Volume Information), reserved storage, the page file, hibernation, NTFS metadata, update leftovers, and folders the scanner was not allowed to read.
- Tools fail because a non-admin scan cannot see inside System Volume Information. Even an admin scan sees shadow-copy storage as one opaque blob. WizTree's notes say the gap between its total and "space used" is mostly reserved space plus NTFS metadata.
- Threads:
  - Microsoft Q&A, [Unexplained Disk Space Usage](https://learn.microsoft.com/en-us/answers/questions/8353ffd8-2ca4-428f-be11-1135f3b4ff9d/unexplained-disk-space-usage?forum=windows-all)
  - Microsoft Q&A, [Sudden 200 GB loss overnight (June 2026)](https://learn.microsoft.com/en-us/answers/questions/5919001/sudden-large-loss-200gb-overnight-c-drive-disk-spa): no cause was confirmed; the answers point to shadow copies.
  - Microsoft support blog, [NTFS misreports free space, part 3](https://learn.microsoft.com/en-us/archive/blogs/supportingwindows/ntfs-misreports-free-space-part-3): shadow-copy diff areas are hidden from folder totals.
  - ElevenForum, [Size of System Volume Information](https://www.elevenforum.com/t/size-of-system-volume-information.7284/): 186 GB in SVI; lowering the System Protection limit fixed it.
  - ElevenForum, [C drive nearly full, why?](https://www.elevenforum.com/t/c-drive-nearly-full-why.5313/page-2)
  - TenForums, [145 GB missing due to System & Reserved](https://www.tenforums.com/performance-maintenance/141582-145gb-missing-hard-drive-space-due-system-reserved.html)
  - AskWoody, [C drive full with system files](https://www.askwoody.com/forums/topic/c-drive-full-with-system-files): System Restore had been set to 100%.
  - SevenForums, [WizTree total vs Explorer mismatch](https://www.sevenforums.com/threads/can-i-get-rid-of-older-win-7-system-volume-information-files.355305/)
  - Reddit (titles and snippets only): [r/Windows11 "Why is my C drive so full?"](https://www.reddit.com/r/Windows11/comments/1cupdku/why_is_my_c_drive_so_full/), [r/windows "250 GB SSD full for no reason"](https://www.reddit.com/r/windows/comments/j807li/my_c_drive_250gb_ssd_is_full_for_no_reason_even/), [r/techsupport "Windows 11 C drive keeps filling up"](https://www.reddit.com/r/techsupport/comments/wuwlxv/windows_11_c_drive_keeps_filling_up_for_no_reason/), [r/computers "120 GB free last night, full today"](https://www.reddit.com/r/computers/comments/1cgne60/my_c_drive_suddenly_is_showing_the_storage_full/), [r/techsupport "C drive full for no reason"](https://www.reddit.com/r/techsupport/comments/njuayn/c_drive_full_for_no_reason/)
- Implication: build a "hidden space" breakdown (candidate C1) and a restore point / shadow storage panel (C15).

### P2. "What is this folder and is it safe to delete?"
- WinSxS, `C:\Windows\Installer` (35-100 GB reports), `ProgramData\Package Cache`, AppData (70-180 GB), CBS logs that loop and fill the drive.
- Threads:
  - Microsoft Q&A on Installer: [Installer files taking almost 100 GB](https://learn.microsoft.com/en-us/answers/questions/5649931/installer-files-taking-almost-100gb), [Cleaning up a 35 GB Installer folder](https://learn.microsoft.com/en-us/answers/questions/3853911/cleaning-up-massive-c-windowsinstaller-(35gb)), [Installer over 90 GB, is there a safe way?](https://learn.microsoft.com/en-us/answers/questions/2777465/my-windows-installer-folder-is-over-90-gb-does-mic), [Installer bloat only getting worse](https://learn.microsoft.com/en-us/answers/questions/5852764/c-windowsinstaller-bloat-issue-only-getting-worse). The consensus: do not delete by hand; uninstalling the owning software is the only supported route; PatchCleaner is the usual third-party answer.
  - WinDirStat [#588 "Integrate with InstallerClean"](https://github.com/windirstat/windirstat/issues/588) (June 2026): the requester reports about 30 GB freed per machine from Adobe installer bloat. A maintainer would rather replicate the feature than integrate it.
  - AppData: [Why is my AppData folder so large?](https://learn.microsoft.com/en-us/answers/questions/2509747/why-is-my-appdata-folder-so-large), [AppData\Local\Packages at 179 GB](https://learn.microsoft.com/en-us/answers/questions/4332666/windows-10-local-appdata-packages-dir-is-at-179gb) (it was a game), [AppData takes 70 GB](https://learn.microsoft.com/en-us/answers/questions/2731214/appdata-takes-up-70gb-how-to-free-up-the-space)
  - Package Cache: Microsoft setup blog, [move the cache with a policy, never delete it by hand](https://devblogs.microsoft.com/setup/?p=3631)
  - CBS: [CBS log is huge and taking up my entire disk](https://learn.microsoft.com/en-us/answers/questions/2787322/cbs-log-is-huge-and-taking-up-my-entire-disk), [Windows folder at 75 GB](https://learn.microsoft.com/en-us/answers/questions/2759680/my-windows-folder-in-my-c-drive-is-huge-75-0-gb-is?forum=windows-windows_10-files)
- Implication: ReClaim's rule texts already answer much of this. Add a read-only report of `Windows\Installer` orphans (C13), and in the space map add an "explain this folder" panel that shows the rule text for known system folders.

### P3. WSL and Docker virtual disks never shrink
- `ext4.vhdx` and `docker_data.vhdx` keep their peak size. `docker system prune` frees space only inside the VM. The fix is shutdown, then trim, then `Optimize-VHD` (needs the Hyper-V module, which Home lacks) or `diskpart compact vdisk`.
- Threads:
  - [microsoft/WSL #4699 "WSL 2 should automatically release disk space back to the host OS"](https://github.com/microsoft/WSL/issues/4699): open since November 2019, about 380 timeline items. The community links a draft `wsl --manage <distro> --compact` (pre-release; [unverified] as shipped).
  - [WSL #40243 proposal: opt-in compaction after shutdown](https://github.com/microsoft/WSL/issues/40243); [WSL discussion #8637: how to move ext4.vhdx](https://github.com/microsoft/WSL/discussions/8637); [WSL #12103: sparse does not work](https://github.com/microsoft/WSL/issues/12103)
  - Microsoft Q&A: [WSL virtual disk compression issue](https://learn.microsoft.com/en-us/answers/questions/1658998/how-do-i-resolve-my-wsl-virtual-disk-compression-i), [sparse VHD storage does not shrink](https://learn.microsoft.com/en-us/answers/questions/1526083/in-wsl2-with-sparse-vhd-the-storage-usage-does-not)
  - Docker forum, [VHDX grows ~1.5 GB after every update, no way to reclaim](https://forums.docker.com/t/docker-desktop-wsl2-vhdx-grows-by-1-5gb-after-every-update-no-way-to-reclaim-space/151729)
  - Blog posts: [Nick Janetakis](https://nickjanetakis.com/blog/reclaiming-tons-of-diskspace-by-compacting-your-docker-desktop-wsl-2-vm); [dev.to, 350 GB reclaimed](https://dev.to/hamada147/docker-on-windows-eating-your-c-drive-how-i-reclaimed-over-350gb-from-bloated-vhdx-files-11dm)
  - Many single-purpose scripts, which is itself a demand signal: [kyto64/WSL2_Disk_Volume_Optimizer](https://github.com/kyto64/WSL2_Disk_Volume_Optimizer), [mtgroupsrls/Shrink-DockerDataVHDX](https://github.com/mtgroupsrls/Shrink-DockerDataVHDX), [AdnanSattar/docker-desktop-wsl-shrink](https://github.com/AdnanSattar/docker-desktop-wsl-shrink)
- Caution: Microsoft's [WSL disk space page](https://learn.microsoft.com/en-us/windows/wsl/disk-space) warns against modifying WSL files in AppData with Windows tools. It documents `wsl --manage --resize` (WSL 2.5+) but not compaction.
- Implication: a guarded compaction wizard (C5) plus Docker prune commands (C12).

### P4. Game shader caches
- A Steam per-game `shadercache` grew from 16 to 41 GB for one game ([Steam Community, No Man's Sky](https://steamcommunity.com/app/275850/discussions/0/3758849893335116921)). NVIDIA DXCache can grow without bound when the size limit is set to Unlimited ([WindowsForum](https://windowsforum.com/news/nvidia-dxcache-using-too-much-disk-space-how-to-clear-shader-cache-safely.446181/), [rtech.support guide](https://rtech.support/guides/clearing-shader-cache/)).
- ReClaim already has rules for the Steam, NVIDIA, AMD, Intel and DirectX shader caches. The gaps are Steam libraries on other drives (on the roadmap) and a per-game breakdown with game names.

### P5. OneDrive and cloud placeholders
- Users run "Free up space" and still see local use. Analyzers count cloud-only files as if they were local.
- Threads: [OneDrive files online-only but still take space](https://learn.microsoft.com/en-us/answers/questions/5275384/onedrive-files-are-on-line-only-but-still-take-up), [OneDrive filling up C:](https://learn.microsoft.com/en-us/answers/questions/5254455/onedrive-is-filling-up-my-computer-c-os-drive), WinDirStat [#50 count cloud-only files as zero](https://github.com/windirstat/windirstat/issues/50) (closed in 2.5 with a "Physical Size" column; a request for a treemap toggle is still open in the comments).
- Implication: C25.

### P6. "Which program is locking this file?"
- PowerToys added File Locksmith for this, and its tracker has several "menu entry missing" reports plus a request to move it to the extended menu: [#23859](https://github.com/microsoft/PowerToys/issues/23859), [#26129](https://github.com/microsoft/PowerToys/issues/26129), [#30448](https://github.com/microsoft/PowerToys/issues/30448). Microsoft Q&A has a [COM Surrogate locking files](https://learn.microsoft.com/en-us/answers/questions/3965962/cant-delete-files-because-they-are-open-in-com-sur?page=2) thread. LockHunter's whole product is this one question.
- Implication: C4.

### P7. Uninstalls leave traces, or leftover scans remove too much
- Missed leftovers: BCU [#561](https://github.com/Klocman/Bulk-Crap-Uninstaller/issues/561) (ProgramData, AppData, dot-folders such as `~/.vscode`, `~/.docker`), [#163](https://github.com/BCUninstaller/Bulk-Crap-Uninstaller/issues/163) (NVIDIA AppData), [#913](https://github.com/BCUninstaller/Bulk-Crap-Uninstaller/issues/913) (Wondershare).
- Over-removal and noise: [#758](https://github.com/Klocman/Bulk-Crap-Uninstaller/issues/758) (a settings folder shared by two Blender versions was deleted), [#262](https://github.com/Klocman/Bulk-Crap-Uninstaller/issues/262) (a false positive, with a request for a whitelist).
- UX: [#703](https://github.com/BCUninstaller/Bulk-Crap-Uninstaller/issues/703) (leftover results lost after pressing ESC).
- Implication: C6, with shared-folder guards, persistence of results and a whitelist.

### P8. "What grew since last week?"
- AskWoody, [Disk space used up: where? when?](https://www.askwoody.com/forums/topic/disk-space-used-up-where-when/page/2): users point to old or paid tools (DiskSCOUT, PA WatchDISK, FolderSizes trend analysis). TreeSize Pro has snapshot comparison. Free tools mostly lack it.
- Implication: C2 (already on the roadmap; the demand confirms its priority).

### P9. Find by type, filter within a folder
- WinDirStat [#706 filter results by extension](https://github.com/windirstat/windirstat/issues/706), [#719 file-types panel should follow the selected folder](https://github.com/windirstat/windirstat/issues/719), [#228](https://github.com/windirstat/windirstat/issues/228) (same request, older), [#401 size histograms and percentiles](https://github.com/windirstat/windirstat/issues/401), [#656 tree view for duplicates](https://github.com/windirstat/windirstat/issues/656), [#634 empty-folder search instead of blind removal](https://github.com/windirstat/windirstat/issues/634).
- Implication: C20.

### P10. App sizes in Settings are wrong
- Reddit snippet ([r/computerhelp](https://www.reddit.com/r/computerhelp/comments/16k7e4c/my_c_drive_is_full_for_no_reason/)): Settings showed Spotify at 291 MB while its offline cache was 12.5 GB, and Steam showed no size at all. The uninstall registry `EstimatedSize` is self-reported, and caches live elsewhere.
- Implication: C7.

### P11. Other recurring large items
- Search index: on Windows 11 it is `Windows.db` (SQLite) and esentutl defragmentation no longer applies; rebuilding the index is the fix ([Pureinfotech](https://pureinfotech.com/fix-large-windowsdb-windows-11/), [WOSHub](https://woshub.com/windows-edb-file-too-big-how-to-reduce-size/)). Candidate C16.
- DriverStore: old NVIDIA packages at about 1 GB each; DriverStore Explorer (RAPR) and `pnputil` are the usual tools ([ElevenForum, is it safe to delete old drivers?](https://www.elevenforum.com/t/driverstore-explorer-safe-to-delete-old-drivers.31591/page-3), [WOSHub](https://woshub.com/how-to-remove-unused-drivers-from-driver-store/)). Candidate C14.
- iPhone and iPad backups: full (not incremental) backups in `Apple\MobileSync\Backup`; Apple recommends deleting them through the Apple Devices app ([Apple support 108809](https://support.apple.com/108809)). Candidate C18.
- Reserved storage (about 7 GB): disabling it is possible but discouraged ([ElevenForum tutorial](https://www.elevenforum.com/t/find-reserved-storage-size-in-windows-11.21395/), [Neowin](https://www.neowin.net/guides/heres-how-to-disable-reserved-storage-to-free-up-space-on-your-windows-pc/)). Show it in C1; do not toggle it.
- Old user profiles ("Other people" in Storage settings). Candidate C17.

### P12. "Analyze this folder" from Explorer
- WizTree, TreeSize and WinDirStat all add an Explorer entry. WinDirStat added per-user registration in 2.8 and a toggle inside the app in 2.5. TreeSize restored its drive entry and added Windows 11 menu support in 4.8 ([TreeSize Free changes](https://jam-software.com/treesize_free/changes.shtml)). Users treat this as table stakes. Candidate C3.

### P13. Distrust of commercial cleaners
- CCleaner bundling, its 2017 malware build and its upsells are the background for open-source alternatives (already covered in `docs/research.md` section 5). Reddit "CCleaner alternative" threads could not be read [unverified]. The implication is unchanged: keep the no-upsell, no-telemetry stance visible in the UI and the README.

---

## 4. Candidate features

Format: **what**; **who has it**; **demand evidence**; **difficulty** (S = days, M = 1-3 weeks, L = more); **risk and safety notes**; **fit** with "safe, honest, no tweaks".

### C1. Hidden space explainer ("Where did the rest go?")
- **What:** For each volume, reconcile `used = scanned allocated + hidden parts + unexplained` and list each part with a plain-language reason and the official way to reduce it:
  - shadow copies / System Restore (used, allocated and maximum size of shadow storage, number of restore points);
  - reserved storage;
  - `pagefile.sys`, `swapfile.sys`, `hiberfil.sys`;
  - NTFS metadata (MFT size);
  - folders that could not be read (count and the elevation hint);
  - WSL and Docker virtual disks;
  - Recycle Bins;
  - cloud placeholders (logical vs local).
- **Who has it:** DaisyDisk (hidden and purgeable space on macOS); WizTree explains the gap in release notes only; TreeSize as admin can see inside SVI. No Windows tool does the full reconciliation [to my knowledge; unverified for paid tools].
- **Demand:** P1. It is the top recurring question across Microsoft Q&A, ElevenForum, TenForums and Reddit titles.
- **Difficulty:** M. Volume totals come from `GetDiskFreeSpaceEx`. MFT size comes from `FSCTL_GET_NTFS_VOLUME_DATA`. Shadow storage comes from WMI `Win32_ShadowStorage` or `vssadmin list shadowstorage` (admin, through the helper, read-only). Reserved storage comes from `fsutil storagereserve query C:` (admin) or DISM state. System file sizes come from directory metadata. Access-denied counts come from the walker.
- **Risk:** Read-only. Actions only open official UIs: System Protection, `powercfg /h off` (already in Tools), Disk Cleanup.
- **Fit:** Excellent. It is the most "honest" feature possible and turns "we cannot see it" into an explanation.

### C2. Scan snapshots and comparison
- **What:** Save a compact scan (per-folder totals). Reopen it instantly. Diff two scans as "grew, shrank, new, gone", sorted by delta, with a treemap colored by growth.
- **Who:** TreeSize Pro (XML and restore-point snapshots), SpaceSniffer (snapshots), WizTree (CSV/MFT re-import), WinDirStat (JSON save and load), FolderSizes (trend database).
- **Demand:** P8; the AskWoody thread; already planned in `ROADMAP.md`.
- **Difficulty:** M (storage format, diff, UI).
- **Risk:** None (read-only). Store snapshots locally only and let the user delete them.
- **Fit:** Excellent.

### C3. Explorer context menu and path argument
- **What:** An "Analyze with ReClaim" entry on folders, drives and folder backgrounds. Launch with a path argument and route it to the running instance through Tauri's single-instance plugin. Make it a per-user toggle in Settings.
- **Who:** WizTree, TreeSize Free and Pro, WinDirStat 2.x.
- **Demand:** P12 (table stakes in all three main competitors).
- **Difficulty:** S for the classic menu (`HKCU\Software\Classes\Directory\shell`, `Drive\shell`, `Directory\Background\shell`). M or L for the Windows 11 top-level menu, which needs an `IExplorerCommand` handler and package identity [unverified details].
- **Risk:** Low. Register per user, remove on uninstall and in portable mode, never touch HKLM.
- **Fit:** Yes.

### C4. Locked-file insight
- **What:** When a delete fails, or on demand, list the processes holding the file or folder through Restart Manager (`RmGetList`), with app name, PID and service name. Offer "Retry", "Open in Task Manager" and possibly a graceful "Ask app to close" (Restart Manager shutdown without force). Never force-kill.
- **Who:** LockHunter, IObit Unlocker, PowerToys File Locksmith, Process Explorer and System Informer (find handle or DLL).
- **Demand:** P6.
- **Difficulty:** S-M. Folders need per-file registration (File Locksmith scans the subtree). Other users' processes need elevation.
- **Risk:** Low if read-only. Graceful close can lose unsaved work, so it needs explicit confirmation; force kill stays out of scope.
- **Fit:** Yes. Already P1-10 in research.md.

### C5. WSL and Docker virtual disk compaction wizard
- **What:** For each distro and Docker disk, show file size against used space inside (`wsl --system -d <distro> df`). Then guide the steps:
  1. Optional Docker cleanup through its CLI.
  2. `wsl --shutdown`, with a check that Docker Desktop is closed.
  3. Optional `fstrim` inside the distro.
  4. Compact with `Optimize-VHD -Mode Full` when the Hyper-V module exists, otherwise a `diskpart` `compact vdisk` script. Use `wsl --manage <distro> --compact` when the installed WSL supports it [unverified].
  5. Report the measured bytes freed.
- **Who:** WinDirStat 2.5 ("optimize virtual hard disk files"), many single-purpose scripts.
- **Demand:** P3. The WSL #4699 thread has been open for 7 years.
- **Difficulty:** M-L (Home vs Pro paths, admin helper, progress, failure handling).
- **Risk:** Medium. Compacting an attached disk can corrupt it, so verify it is detached and nothing is running. Compaction needs free space on the host. Microsoft advises against touching WSL files with Windows tools, so show the exact commands, recommend `wsl --export` first, and never delete the file.
- **Fit:** Yes, as an official-tool wizard with a preview and the PowerShell export.

### C6. Leftover scan after uninstall, with confidence levels
- **What:**
  - Search AppData Roaming and Local, LocalLow, ProgramData, `%USERPROFILE%\.<name>` dot-folders, Start Menu shortcuts and the leftover install folder.
  - Score by exact match on DisplayName, Publisher and InstallLocation.
  - Preselect only high confidence. Never preselect folders shared by other installed apps from the same vendor.
  - Persist results until dismissed and offer a per-path "never suggest again" list.
  - Registry: at most the app's own `HKCU\Software\<Vendor>\<App>` key, as Review, with a `.reg` export first. No class or CLSID sweeps.
- **Who:** Revo, Geek, BCU.
- **Demand:** P7.
- **Difficulty:** M.
- **Risk:** Medium. The BCU #758 case (a shared folder deleted) is the trap, and the matching must account for it. Send files to the Recycle Bin.
- **Fit:** Yes for files. Registry only in the narrow, exported form, otherwise it drifts into registry cleaning.

### C7. True app footprint
- **What:** For each installed app, show install folder + matched AppData and ProgramData + caches matched by rules = real total, next to the self-reported registry size. Sort apps by real total.
- **Who:** Geek and BCU show sizes. None of the reviewed tools combines them with caches [to my knowledge].
- **Demand:** P10.
- **Difficulty:** M (matching is shared with C6).
- **Risk:** Read-only.
- **Fit:** Excellent (honest numbers).

### C8. Batch and quiet uninstall
- **What:** Select several apps, use `QuietUninstallString` or MSI quiet mode where available, run them one at a time with progress, then offer one leftover review at the end.
- **Who:** BCU (core feature), Revo (multiple uninstall, Pro) [unverified].
- **Demand:** BCU's popularity; BCU #447 asks for an "always run as admin" option.
- **Difficulty:** M.
- **Risk:** Medium. A silent uninstall can remove shared runtimes, so show what each uninstaller is, and take a restore point first (C19).
- **Fit:** Yes.

### C9. Forced removal of broken uninstall entries
- **What:** For entries whose uninstaller and install folder are both missing, offer "remove from list" (delete that one Uninstall key after a `.reg` export) plus the leftover scan.
- **Who:** Revo, Geek, BCU.
- **Demand:** Common "program won't uninstall" questions [Reddit unverified].
- **Difficulty:** M.
- **Risk:** Medium-high if extended to "force uninstall" of apps that still exist. Limit it to entries that are provably dead.
- **Fit:** Partial. Narrow scope only.

### C10. Uninstall by window, shortcut or folder (Hunter-style)
- **What:** Pick a window (or drop a shortcut or folder), resolve it to its exe, then match it to an installed app.
- **Who:** Revo (Hunter mode), BCU.
- **Demand:** Helps with "what app is this tray icon?" Moderate.
- **Difficulty:** S-M.
- **Risk:** Low (only selects; uninstall goes through the normal flow).
- **Fit:** Yes.

### C11. Steam libraries and per-game shader caches
- **What:**
  - Read `libraryfolders.vdf` (already on the roadmap) and each library's `steamapps\shadercache\<appid>`.
  - Show game names from the `appmanifest_<appid>.acf` files.
  - Flag shader caches of uninstalled games as Safe and those of installed games as Review.
- **Who:** None of the reviewed tools does a per-game breakdown [to my knowledge].
- **Demand:** P4.
- **Difficulty:** S.
- **Risk:** Low (regenerable; warn about stutter on first launch).
- **Fit:** Yes.

### C12. Package-manager and Docker cleanup through their own commands
- **What:** When the tool is on PATH, run its own command:
  - `npm cache clean --force`, `pnpm store prune`, `yarn cache clean`
  - `pip cache purge`, `uv cache prune`
  - `dotnet nuget locals all --clear`, `go clean -cache`
  - Docker: `docker builder prune` and `docker image prune` (never volumes), and `docker system df` for the preview.

  Fall back to the existing folder rules otherwise.
- **Who:** Mole (clean and purge), npkill profiles.
- **Demand:** P3 and the dev audience; already P1-9 in research.md.
- **Difficulty:** M.
- **Risk:** Low-medium. Commands may need the tool closed. Docker volumes contain data and are never pruned.
- **Fit:** Yes.

### C13. `Windows\Installer` orphan report
- **What:** Use the MSI API (`MsiEnumProductsEx`, `MsiEnumPatchesEx`, local package paths) to build the registered set, then list unregistered `.msi` and `.msp` files and their size. Read-only by default. Optionally "move to a quarantine folder on another drive" with a one-click restore (PatchCleaner's own recommendation). Never delete.
- **Who:** PatchCleaner, InstallerClean; requested in WinDirStat #588.
- **Demand:** P2 (multiple 35-100 GB threads).
- **Difficulty:** M.
- **Risk:** High if acted on. False positives break repair, patching and uninstall, and Microsoft supports only uninstalling the software.
- **Fit:** The report fits well (it is honest and answers the question). Quarantine only in expert mode. Deletion does not fit.

### C14. Old driver packages in the DriverStore
- **What:** Parse `pnputil /enum-drivers`, group by original INF, provider and class, and propose older versions of the same package. Remove them with `pnputil /delete-driver` without `/force`, which refuses packages in use. Exclude boot-critical classes.
- **Who:** DriverStore Explorer (RAPR).
- **Demand:** P11. NVIDIA packages are about 1 GB each.
- **Difficulty:** M.
- **Risk:** Medium-high. A mistake can break devices or rollback, and RAPR warns about boot failure. Create a restore point and keep the newest two versions.
- **Fit:** Expert mode only, through the official tool.

### C15. Restore points and shadow storage panel
- **What:** List restore points (date, type) and show shadow storage used and maximum. Buttons: open System Protection to change the maximum; Disk Cleanup "More Options > System Restore and Shadow Copies" to keep only the newest point [unverified on current builds].
- **Who:** Windows built-ins only. Cleaners rarely surface it.
- **Demand:** P1 (most ElevenForum and AskWoody cases ended here).
- **Difficulty:** S-M (WMI read through the helper).
- **Risk:** Low (official UIs). Explain that deleting restore points removes rollback.
- **Fit:** Excellent.

### C16. Search index size
- **What:** Show the size of `Windows.db` or `Windows.edb` and explain it. Open Indexing Options for "Rebuild" and suggest excluding big PST or archive folders.
- **Who:** None of the reviewed tools.
- **Demand:** P11.
- **Difficulty:** S.
- **Risk:** Low. A rebuild takes hours and search is degraded meanwhile; say so.
- **Fit:** Yes.

### C17. User profile sizes
- **What:** List profiles (ProfileList registry) with size and last use. Link to the official removal UI (System Properties > User Profiles, or Settings > Accounts).
- **Who:** Windows Storage settings ("Other people").
- **Demand:** P11.
- **Difficulty:** S (other profiles need admin).
- **Risk:** Read-only.
- **Fit:** Yes.

### C18. Device backups
- **What:** Show iPhone and iPad backups in `%USERPROFILE%\Apple\MobileSync\Backup` and `%APPDATA%\Apple Computer\MobileSync\Backup` with device name and date (from `Info.plist`). Review risk; recommend deleting through Apple Devices.
- **Who:** CleanMyMac on macOS [unverified for Windows tools].
- **Demand:** P11, Apple support page, Microsoft Q&A AppData answer.
- **Difficulty:** S.
- **Risk:** User data, so never preselect, and warn about encrypted-backup passwords.
- **Fit:** Yes as Review.

### C19. Restore point before uninstall and other system changes
- **What:** `Checkpoint-Computer` / `SRSetRestorePoint` before uninstall batches, startup changes in HKLM and driver removal. Say when the 24-hour limit skipped it.
- **Who:** Revo, BCU [unverified for BCU].
- **Difficulty:** S.
- **Risk:** Low.
- **Fit:** Yes. Already P1-11 in research.md.

### C20. Filter syntax, file-type filter, file age view
- **What:** Filters like `*.iso >1GB older:1y type:video`, type chips that follow the selected folder, and size and age histograms.
- **Who:** SpaceSniffer, WinDirStat 2.6-2.9, Everything, WizTree.
- **Demand:** P9.
- **Difficulty:** M.
- **Risk:** None.
- **Fit:** Yes. Already on the roadmap.

### C21. Export reports
- **What:** CSV and JSON (and a simple HTML) export for the space map, large files, duplicates, apps, startup entries and cleanup history.
- **Who:** WizTree, WinDirStat, TreeSize Pro, Geek (HTML apps list), BCU.
- **Demand:** Sysadmin and help-desk use; Geek reviews cite the HTML apps report.
- **Difficulty:** S.
- **Risk:** Exported paths can reveal user data, so save only where the user chooses.
- **Fit:** Yes.

### C22. Command-line interface
- **What:** `reclaim scan`, `reclaim clean --dry-run`, `--json`, built on the core crate.
- **Who:** WizTree, WinDirStat, TreeSize Pro, Mole, Everything.
- **Difficulty:** M.
- **Risk:** Destructive commands need `--yes`.
- **Fit:** Yes (roadmap).

### C23. MFT fast scan
- **What:** When elevated on local NTFS, enumerate through the MFT or USN data, with the current walker as fallback.
- **Who:** WizTree, WinDirStat 2.5, Everything.
- **Demand:** Speed is WizTree's main selling point.
- **Difficulty:** L (correctness for hard links, ADS, reparse points and permissions).
- **Risk:** Read-only.
- **Fit:** Yes (roadmap "Later").

### C24. Cleanup basket (staging area)
- **What:** One basket that collects items from the space map, large files, duplicates, artifacts and junk. It dedupes nested paths, shows the total, and produces one preview plan and one confirmation.
- **Who:** DaisyDisk (collector, with a 5-second cancel); CleanMyMac partly.
- **Demand:** UX gap; users bounce between pages today [inferred, not from a thread].
- **Difficulty:** M.
- **Risk:** Lowers risk (one reviewed plan).
- **Fit:** Excellent; it extends "preview is the default path".

### C25. Cloud-only awareness
- **What:** A logical vs local size toggle in the treemap, a placeholder badge, and a OneDrive summary (local, online-only, pinned). A link to "Free up space"; optionally run the documented `attrib +U -P` on a folder the user chooses [unverified syntax].
- **Who:** WinDirStat (physical size column), Storage Sense (dehydrates after N days).
- **Demand:** P5, WinDirStat #50.
- **Difficulty:** S-M.
- **Risk:** Never hydrate. Dehydrating is reversible but needs a connection to reopen files.
- **Fit:** Yes.

### C26. NTFS compression for cold folders and games
- **What:** Estimate the savings, compress with `compact /C /EXE:LZX` (or XPRESS), offer a one-click decompress, and show the ratio afterwards. Never CompactOS by default.
- **Who:** WinDirStat, CompactGUI, TreeSize [unverified].
- **Demand:** CompactGUI's popularity with gamers [stars count unverified]; Ghacks coverage.
- **Difficulty:** M.
- **Risk:** CPU cost on access. Some games patch files and decompress them. Do not apply to OS folders.
- **Fit:** Borderline. It is reversible and space-related, but closer to a "tweak". Expert mode only.

### C27. Replace duplicates with hard links
- **What:** On the same NTFS volume, replace confirmed duplicates with hard links.
- **Who:** TreeSize Pro, Czkawka; WinDirStat has related fixes.
- **Difficulty:** M.
- **Risk:** Editing one copy changes all copies; some apps break links.
- **Fit:** Expert only (roadmap P2).

### C28. Similar images
- **What:** Perceptual hashes with a BK-tree.
- **Who:** Czkawka, CleanMyMac.
- **Difficulty:** L.
- **Fit:** Yes (roadmap).

### C29. Owner column and per-user breakdown
- **Who:** WinDirStat, TreeSize Pro.
- **Difficulty:** S-M.
- **Fit:** Yes, low priority.

### C30. Startup coverage: read-only tasks, services and signatures
- **What:** Show non-Microsoft scheduled tasks and auto-start services with signature status (`WinVerifyTrust`) and a "target missing" flag. Toggles only where Windows itself offers them (StartupApproved); scheduled tasks and services are opened in their own consoles.
- **Who:** Autoruns, CCleaner, BCU.
- **Difficulty:** M.
- **Risk:** Disabling services is a tweak, so keep this read-only.
- **Fit:** Yes as read-only.

### C31. Relocation guide for stores that support moving
- **What:** For big stores that support relocation, show the official way:
  - Ollama (`OLLAMA_MODELS`), Hugging Face (`HF_HOME`)
  - npm, pnpm, pip and NuGet cache settings
  - Docker Desktop disk image location
  - Steam "move install folder"
  - WSL export and import
  - VS Package Cache policy

  Export as a commented PowerShell script; do not apply silently.
- **Who:** Nobody bundles this [to my knowledge]. Users rely on blog posts.
- **Demand:** Ollama and WSL move threads; WSL discussion #8637.
- **Difficulty:** M.
- **Risk:** Changing env vars is persistent configuration, so keep it script-only or behind explicit confirmation.
- **Fit:** Yes as guidance.

### C32. Old installer and archive finder
- **What:** `.msi`, setup `.exe`, `.iso`, and `.zip` files that have an extracted sibling folder, in Downloads and Desktop, older than N days, as Review.
- **Who:** Mole `installer`, PC Manager (Downloads), Storage Sense (Downloads by age).
- **Difficulty:** S.
- **Fit:** Yes (roadmap P1-8).

### C33. Duplicate folders
- **What:** Whole-folder duplicates (same tree hash).
- **Who:** dupeGuru and AllDup [unverified].
- **Demand:** DataHoarder [unverified, Reddit not read].
- **Difficulty:** M.
- **Fit:** Yes, low priority.

### C34. Live updates and C35. Alternative visualizations
- WinDirStat, SpaceSniffer and WizTree refresh live; WinDirStat 2.8 and DaisyDisk offer sunburst and flame views.
- Difficulty M each. Low priority.

### C36. Delete on reboot for locked items
- **Who:** LockHunter.
- **Difficulty:** S (`MoveFileEx` with `MOVEFILE_DELAY_UNTIL_REBOOT`).
- **Risk:** Moderate (invisible pending deletes).
- **Fit:** Expert only, per item, listed in history.

### C37. Chrome and Edge on-device AI model folders
- **What:** An AI-group rule for browser-downloaded on-device model weights (for example `OptGuideOnDeviceModel`, reported at several GB).
- **Difficulty:** S.
- **Fit:** Yes, but **[unverified]**: the folder name, size and whether the browser re-downloads it could not be checked before the search budget ran out.

---

## 5. Ranking (value / effort)

Value 1-5 (demand x space reclaimed x trust gained); effort S = 1, M = 2, L = 3.

| Rank | ID | Feature | Value | Effort | Note |
|---|---|---|---|---|---|
| 1 | C1 | Hidden space explainer | 5 | M | Answers the most common question; read-only |
| 2 | C3 | Explorer context menu + path argument | 4 | S | Table stakes in WizTree, TreeSize and WinDirStat |
| 3 | C25 | Cloud-only awareness | 4 | S-M | Fixes wrong totals for OneDrive users |
| 4 | C4 | Locked-file insight (Restart Manager) | 4 | S-M | A standalone-tool problem, solved in place |
| 5 | C15 | Restore points and shadow storage panel | 4 | S-M | Usual root cause of "missing" space |
| 6 | C2 | Scan snapshots and comparison | 5 | M | "What grew?"; few free tools do it |
| 7 | C11 | Steam libraries + per-game shader caches | 3 | S | Already planned; adds game names |
| 8 | C32 | Old installer and archive finder | 3 | S | Easy, frequent win |
| 9 | C21 | Export reports | 3 | S | Help-desk and sysadmin use |
| 10 | C16 | Search index size + rebuild link | 3 | S | Explains a multi-GB mystery file |
| 11 | C17 | User profile sizes | 3 | S | Explains "Other people" |
| 12 | C18 | Device backups (MobileSync) | 3 | S | Large, forgotten, user-owned |
| 13 | C6 | Leftover scan with confidence | 5 | M | Main uninstaller gap; guard shared folders |
| 14 | C7 | True app footprint | 4 | M | Honest sizes vs Settings |
| 15 | C20 | Filter syntax, type filter, age view | 4 | M | Repeated WinDirStat requests |
| 16 | C24 | Cleanup basket | 4 | M | Safer, simpler multi-page cleanup |
| 17 | C12 | Package-manager + Docker commands | 4 | M | Dev audience; official commands |
| 18 | C5 | WSL/Docker VHDX compaction wizard | 5 | M-L | Big wins; needs strong guards |
| 19 | C19 | Restore point before uninstall | 3 | S | Prerequisite for C8 and C14 |
| 20 | C10 | Uninstall by window / shortcut / folder | 3 | S-M | Revo's best-known UX |
| 21 | C8 | Batch + quiet uninstall | 3 | M | BCU's core value |
| 22 | C31 | Relocation guide for movable stores | 3 | M | Moves models and caches off C: safely |
| 23 | C13 | Windows\Installer orphan report | 4 | M | Report-only; quarantine in expert mode |
| 24 | C30 | Read-only scheduled tasks/services + signatures | 3 | M | Autoruns-lite without tweaks |
| 25 | C23 | MFT fast scan | 4 | L | Speed parity with WizTree |
| 26 | C22 | CLI | 3 | M | Roadmap; PowerShell export covers part of it |
| 27 | C14 | DriverStore old packages | 3 | M | Expert; official pnputil only |
| 28 | C26 | NTFS compression | 3 | M | Expert; borderline tweak |
| 29 | C27 | Hard-link dedupe | 2 | M | Expert |
| 30 | C9 | Remove dead uninstall entries | 2 | M | Narrow scope |
| - | C28, C29, C33-C37 | Similar images, owner column, duplicate folders, live view, sunburst, delete on reboot, browser AI models | 1-3 | S-L | Later |

---

## 6. Do not add

| Feature | Who has it | Why not |
|---|---|---|
| Registry cleaner, including broad "registry leftovers" sweeps | CCleaner, Wise, Glary, PrivaZer, Revo (traces) | Microsoft does not support registry cleaners (KB 2563254); no measurable space; real breakage risk. Only the app's own key, exported first (C6) |
| RAM optimizer, "Performance Optimizer"/sleep mode, one-click "Boost" | CCleaner, Wise Care, Glary, PC Manager | Not disk space; effects are temporary or placebo; puts apps to sleep that need to run (reviewers warn about OneDrive) |
| Driver updater / software updater | CCleaner (paid), Wise, Glary | Out of scope; network and trust surface; a classic upsell; driver installs are high risk. winget and Windows Update exist |
| Secure shredding, free-space wipe, MFT/USN trace scrubbing | PrivaZer, BleachBit, CCleaner Drive Wiper, Revo Evidence Remover | Not reliable on SSDs; extra SSD writes; low-level metadata scrubbing can damage the file system; no space gain. Already "not planned" |
| Generic "move this program / AppData to another drive" with junctions | Steam Mover, FreeMove, JunctionMaster | Breaks updates, repairs and uninstallers; Store apps cannot move; the WSL docs warn against touching its files. Use the official relocation in C31 |

Also keep out, consistent with `ROADMAP.md`:
- forced unlock that closes handles or unloads DLLs (IObit Unlocker style), and automatic process killing;
- deleted-file recovery (WinDirStat 2.9, DiskGenius): a different product with low success on TRIM SSDs;
- privacy-trace cleaning of cookies and history: little space, high regret;
- scheduled automatic deletion;
- disabling reserved storage, CompactOS or `DISM /ResetBase` by default.

---

## 7. Unverified items and open questions

- Reddit thread contents (only titles and snippets from search results were seen). Reddit demand for "CCleaner alternative", "duplicate folders", "move AppData" and DataHoarder topics is assumed, not confirmed.
- `wsl --manage <distro> --compact`: seen only as a draft PR / pre-release mentioned by community comments on WSL #4699.
- Disk Cleanup "More Options > System Restore and Shadow Copies" on current Windows 11 builds.
- `attrib +U -P` for OneDrive dehydration (from memory; check Microsoft's Files On-Demand docs).
- CCleaner Duplicate Finder and Drive Wiper in current versions; Wise Disk Cleaner scheduling, "system slimming" and defrag; Revo batch uninstall; BCU restore points and backups; TreeSize MFT scanning and treemap in Free; Everything `dm:` syntax and `es.exe`.
- Windows 11 top-level context menu requirements (IExplorerCommand plus package identity).
- Chrome and Edge on-device model folder names and re-download behavior (C37).
- Whether `Win32_ShadowStorage` and `fsutil storagereserve query` need elevation on 24H2/25H2 (assume yes; use the helper).

---

## Sources

Tools and vendor docs
- WinDirStat CHANGELOG: https://github.com/windirstat/windirstat/blob/master/CHANGELOG.md
- WinDirStat 2.5 news (AlternativeTo): https://alternativeto.net/news/2026/1/windirstat-2-5-adds-dark-mode-ntfs-mft-scanning-improved-search-and-new-file-actions/
- WinDirStat issues: https://github.com/windirstat/windirstat/issues (#50, #228, #401, #588, #634, #656, #706, #719)
- WizTree (Neowin release notes): https://www.neowin.net/software/wiztree-431/ ; analysis guide: https://www.luisllamas.es/en/wiztree/
- TreeSize features: https://jam-software.com/treesize/features.shtml ; manual (Pro): https://manuals.jam-software.com/treesize_free/EN/treesize_professional__persona.html ; Free changes: https://jam-software.com/treesize_free/changes.shtml
- SpaceSniffer (Chocolatey): https://community.chocolatey.org/packages/spacesniffer
- Everything indexes: https://www.voidtools.com/support/everything/indexes
- DaisyDisk guide: https://daisydiskapp.com/guide/purgeable-space ; https://daisydiskapp.com/guide/deleting-files
- Mole: https://github.com/tw93/Mole
- Czkawka / Krokiet: https://github.com/qarmin/czkawka ; issues #1293, #2079
- BCUninstaller: https://www.bcuninstaller.com/ ; https://github.com/BCUninstaller/Bulk-Crap-Uninstaller ; issues #163, #262, #447, #561, #703, #758, #913
- Geek Uninstaller: https://geekuninstaller.com/ ; ghacks: https://www.ghacks.net/?p=122629
- Revo Uninstaller (secondary): https://windowsreport.com/revo-uninstaller-pro-review/ ; https://www.ghacks.net/?p=2799
- Autoruns: https://learn.microsoft.com/en-us/sysinternals/downloads/autoruns
- File Locksmith: https://learn.microsoft.com/en-us/windows/powertoys/file-locksmith ; PowerToys issues #23859, #26129, #30448
- LockHunter (secondary): https://www.neowin.net/amp/lockhunter-334/
- PrivaZer user guide: https://privazer.com/PrivaZer-User-Guide.pdf ; BetaNews: https://betanews.com/article/privazer-is-one-of-the-most-powerful-free-privacy-tools-weve-ever-seen/
- Glary Utilities (Neowin): https://www.neowin.net/software/glary-utilities-25501790/
- Wise Care 365 (TechSpot): https://www.techspot.com/downloads/5539-wise-care-365.html
- CCleaner (secondary): https://thesweetbits.com/is-ccleaner-safe-a-review-of-ccleaner/ ; https://www.ghacks.net/?p=176062
- Microsoft PC Manager: https://pcmanager.microsoft.com/en-us ; Windows Latest: https://www.windowslatest.com/2024/05/11/microsoft-pc-manager-performance-booster-for-windows-11-gets-files-cleanup/ ; hands-on: https://www.adamfowlerit.com/2024/05/hands-on-with-microsofts-own-pc-cleaner-and-utility-toolkit-microsoft-pc-manager/
- Storage Sense cleanup recommendations: https://www.windowslatest.com/2020/04/09/windows-10-will-help-you-free-up-space-with-recommendations/
- BleachBit releases and issues: https://github.com/bleachbit/bleachbit/releases ; #1395, #1396
- PatchCleaner: https://www.homedev.com.au/Free/PatchCleaner
- DriverStore: https://woshub.com/how-to-remove-unused-drivers-from-driver-store/ ; https://www.elevenforum.com/t/driverstore-explorer-safe-to-delete-old-drivers.31591/page-3
- compact / CompactGUI: https://www.ghacks.net/?p=137724 ; https://www.ghacks.net/?p=178574
- FolderSizes trend analysis: https://www.foldersizes.com/Screens/TrendAnalysis

Windows docs and official guidance
- WSL disk space: https://learn.microsoft.com/en-us/windows/wsl/disk-space
- WSL issues: https://github.com/microsoft/WSL/issues/4699 ; https://github.com/microsoft/WSL/issues/40243 ; https://github.com/microsoft/WSL/discussions/8637 ; https://github.com/microsoft/WSL/issues/12103
- Package Cache: https://devblogs.microsoft.com/setup/?p=3631
- NTFS misreports free space: https://learn.microsoft.com/en-us/archive/blogs/supportingwindows/ntfs-misreports-free-space-part-3
- Reserved storage: https://www.elevenforum.com/t/find-reserved-storage-size-in-windows-11.21395/ ; https://www.neowin.net/guides/heres-how-to-disable-reserved-storage-to-free-up-space-on-your-windows-pc/
- Search index: https://pureinfotech.com/fix-large-windowsdb-windows-11/ ; https://woshub.com/windows-edb-file-too-big-how-to-reduce-size/
- Apple backups: https://support.apple.com/108809

User threads (Microsoft Q&A, forums)
- https://learn.microsoft.com/en-us/answers/questions/8353ffd8-2ca4-428f-be11-1135f3b4ff9d/unexplained-disk-space-usage?forum=windows-all
- https://learn.microsoft.com/en-us/answers/questions/5919001/sudden-large-loss-200gb-overnight-c-drive-disk-spa
- https://learn.microsoft.com/en-ie/answers/questions/5941692/my-boot-drive-c-is-almost-full-what-files-are-safe
- https://learn.microsoft.com/en-us/answers/questions/5649931/installer-files-taking-almost-100gb
- https://learn.microsoft.com/en-us/answers/questions/3853911/cleaning-up-massive-c-windowsinstaller-(35gb)
- https://learn.microsoft.com/en-us/answers/questions/2777465/my-windows-installer-folder-is-over-90-gb-does-mic
- https://learn.microsoft.com/en-us/answers/questions/5852764/c-windowsinstaller-bloat-issue-only-getting-worse
- https://learn.microsoft.com/en-us/answers/questions/2509747/why-is-my-appdata-folder-so-large
- https://learn.microsoft.com/en-us/answers/questions/4332666/windows-10-local-appdata-packages-dir-is-at-179gb
- https://learn.microsoft.com/en-us/answers/questions/2731214/appdata-takes-up-70gb-how-to-free-up-the-space
- https://learn.microsoft.com/en-us/answers/questions/5275384/onedrive-files-are-on-line-only-but-still-take-up
- https://learn.microsoft.com/en-us/answers/questions/5254455/onedrive-is-filling-up-my-computer-c-os-drive
- https://learn.microsoft.com/en-us/answers/questions/2787322/cbs-log-is-huge-and-taking-up-my-entire-disk
- https://learn.microsoft.com/en-us/answers/questions/2759680/my-windows-folder-in-my-c-drive-is-huge-75-0-gb-is?forum=windows-windows_10-files
- https://learn.microsoft.com/en-us/answers/questions/1658998/how-do-i-resolve-my-wsl-virtual-disk-compression-i
- https://learn.microsoft.com/en-us/answers/questions/1526083/in-wsl2-with-sparse-vhd-the-storage-usage-does-not
- https://learn.microsoft.com/en-us/answers/questions/3965962/cant-delete-files-because-they-are-open-in-com-sur?page=2
- https://www.elevenforum.com/t/size-of-system-volume-information.7284/
- https://www.elevenforum.com/t/c-drive-nearly-full-why.5313/page-2
- https://www.elevenforum.com/t/c-drive-filling-up-fast.18981/
- https://www.tenforums.com/performance-maintenance/141582-145gb-missing-hard-drive-space-due-system-reserved.html
- https://www.tenforums.com/general-support/81162-hard-drive-filled-invisible-files.html
- https://www.askwoody.com/forums/topic/c-drive-full-with-system-files
- https://www.askwoody.com/forums/topic/disk-space-used-up-where-when/page/2
- https://www.sevenforums.com/threads/can-i-get-rid-of-older-win-7-system-volume-information-files.355305/
- https://steamcommunity.com/app/275850/discussions/0/3758849893335116921
- https://windowsforum.com/news/nvidia-dxcache-using-too-much-disk-space-how-to-clear-shader-cache-safely.446181/
- https://rtech.support/guides/clearing-shader-cache/
- https://forums.docker.com/t/docker-desktop-wsl2-vhdx-grows-by-1-5gb-after-every-update-no-way-to-reclaim-space/151729
- https://nickjanetakis.com/blog/reclaiming-tons-of-diskspace-by-compacting-your-docker-desktop-wsl-2-vm
- https://dev.to/hamada147/docker-on-windows-eating-your-c-drive-how-i-reclaimed-over-350gb-from-bloated-vhdx-files-11dm

Reddit (titles and snippets from search results only; not read)
- https://www.reddit.com/r/Windows11/comments/1cupdku/why_is_my_c_drive_so_full/
- https://www.reddit.com/r/computerhelp/comments/16k7e4c/my_c_drive_is_full_for_no_reason/
- https://www.reddit.com/r/techsupport/comments/1ainahp/why_is_my_c_drive_so_unnecessarily_full_and_how/
- https://www.reddit.com/r/computers/comments/1cgne60/my_c_drive_suddenly_is_showing_the_storage_full/
- https://www.reddit.com/r/windows/comments/jmx1p7/c_drive_full_for_no_reason/
- https://www.reddit.com/r/techsupport/comments/wuwlxv/windows_11_c_drive_keeps_filling_up_for_no_reason/
- https://www.reddit.com/r/windows/comments/j807li/my_c_drive_250gb_ssd_is_full_for_no_reason_even/
- https://www.reddit.com/r/techsupport/comments/njuayn/c_drive_full_for_no_reason/
