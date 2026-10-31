# Linux: what to protect, what to scan, what to clean

Date: 2026-11-01. Input for the Linux port, phase 1 (phase 0, the `cfg(windows)` split, shipped in 0.3.0). Sources: man pages, freedesktop and UAPI specs, distro and vendor docs, and the source code of other Linux tools on GitHub. GPL projects (BleachBit, safe-rm, Stacer, QDirStat, filelight, Winapp2) were read for ideas only; no list or rule file was copied from them. Claims that could not be checked against a primary source are marked **[unverified]**.

## Summary

1. **The guard comes first.** Until the Linux protected lists below exist, the engine refuses every delete off Windows (`Error::Unsupported`). They are the first phase 1 task.
2. **Compare paths byte-exact.** Linux paths are case-sensitive and need not be UTF-8. `paths::key()` lowercases and turns `/` into `\`; on Linux it must keep the `OsStr` as is.
3. **Read the mount table before scanning.** `/proc/self/mountinfo` decides what a scan may enter. Kernel filesystems are never entered. Network and FUSE mounts are the Linux form of online-only cloud files: they are shown but not entered. The scan stays on one mount by default.
4. **Delete with `openat` and `unlinkat` relative to a trusted folder, with an `st_dev` check.** `std::fs::remove_dir_all` does not follow symlinks (since Rust 1.58.1), but it does descend into mounts, so the project keeps its own walker.
5. **User caches are safe; system areas get report rules.** Almost every user-level cache lives under `~/.cache`, `~/.config/<Electron app>` or a tool folder in `$HOME`, and needs no root. Package caches, the journal, snaps, Flatpak runtimes and containers are measured and offered with their official command, never deleted as files.
6. **No elevated GUI, and no paths sent to `pkexec`.** An AppImage cannot be elevated with `pkexec` (root cannot read the user's FUSE mount), and sending paths to an elevated `rm` is exactly what the safety model forbids. The model to follow is GNOME Disks: an unprivileged app plus the privileged services that already exist.
7. **Ship AppImage plus deb/rpm, not Flatpak.** A sandboxed cleaner cannot see most of what it should measure.

## 1. The guard on Linux

Paths come from the environment and the XDG files, are made absolute, and are compared as exact bytes after resolving the parent's real path. The link itself is checked, not its target, the way safe-rm does it.

### A. Nothing inside may be deleted (the Windows `subtree` list)

| Path | Why |
|---|---|
| `/boot`, `/efi`, `/boot/efi` | Kernels and bootloader |
| `/usr`, and `/bin`, `/sbin`, `/lib`, `/lib32`, `/lib64`, `/libx32` (often links into `/usr`) | Owned by the package manager |
| `/etc` | System configuration |
| `/proc`, `/sys`, `/dev`, `/run` | Kernel interfaces, not files |
| `/var/lib` | Package databases (dpkg, rpm, pacman), Flatpak, snapd, Docker. Cleaned only through report rules |
| `/snap` | Read-only squashfs mounts |
| `lost+found` on any filesystem | fsck recovery |
| `/nix/store`, `/sysroot`, `/ostree` | NixOS and Fedora Atomic system trees **[proposed, not researched in depth]** |
| Every `.git` folder, the app's own data folder | Same as on Windows |

### B. May not be removed or emptied as a whole (rules may target named children)

- `/`, `/home`, `$HOME`, `/root`, `/opt`, `/srv`, `/usr/local`.
- `/var`, `/var/cache`, `/var/log`, `/var/tmp`, `/tmp`, `/var/spool`, `/var/mail`, `/var/crash`.
- Mount parents: `/media`, `/mnt`, `/run/media/$USER`, and every mount point found in mountinfo.
- XDG folders: `~/.config`, `~/.local`, `~/.local/share`, `~/.local/state`, `~/.local/bin`, `~/.cache` and `$XDG_RUNTIME_DIR`. Rules name `~/.cache/<app>`, never `~/.cache` itself (Stacer offers all of `~/.cache` at once, which is what to avoid). `~/.cache/doc` is Flatpak's document portal mount and is never entered.
- The eight user folders from `~/.config/user-dirs.dirs` (DESKTOP, DOWNLOAD, TEMPLATES, PUBLICSHARE, DOCUMENTS, MUSIC, PICTURES, VIDEOS). Their names are often translated, so they are read from that file, never assumed in English. An entry that is just `$HOME` is skipped. This file is the Linux counterpart of Windows Known Folders.
- Trash: `~/.local/share/Trash`, `$topdir/.Trash`, `$topdir/.Trash-$uid`. Only an "empty trash" action that follows the spec may remove `files/*` together with the matching `info/*.trashinfo`.
- Package roots: `~/.var`, `~/.var/app`, `~/.var/app/<id>`, `~/snap`, `~/snap/<name>`, `~/.local/share/flatpak`.

### C. Precious user data

These folders are never inside a rule. Duplicates and Large files also skip them by default.

- Keys and passwords: `~/.ssh`, `~/.gnupg`, `~/.password-store` (or `$PASSWORD_STORE_DIR`), `~/.local/share/keyrings`, `~/.gnome2/keyrings`, `~/.local/share/kwalletd` **[unverified]**, `~/.pki` **[unverified]**.
- Browser profiles: `~/.mozilla`, `~/.config/mozilla`, `~/.config/google-chrome`, `~/.config/chromium`, `~/.config/BraveSoftware`, `~/.config/microsoft-edge`, `~/.config/vivaldi`. On Linux these browsers keep their HTTP cache in `~/.cache/<browser>`, so the profile can be protected whole. Exception: Service Worker CacheStorage stays inside the profile and needs a `review` rule that names that exact subfolder.
- Mail: `~/.thunderbird`, `~/.local/share/evolution`.
- Flatpak `~/.var/app/*/{config,data,.local/state}` and Snap `~/snap/*/*`, except their cache folders.
- Wallets such as `~/.bitcoin` **[unverified]**.

Czkawka's defaults for its "your files" tools are a useful second opinion: it skips `.git`, `node_modules`, `lost+found`, trash folders, `~/.cache`, `~/.var/app` and hidden folders in the home folder.

## 2. Scan policy

1. **Read the mount table first.** Parse `/proc/self/mountinfo` at scan start: mount id, parent id, `major:minor` (equals `st_dev`), root, mount point, filesystem type, source. Decode the octal escapes (`\040` for a space). A root other than `/` is a bind mount, which keeps its source's `st_dev`, so mountinfo is needed to see it. `statfs` reports FUSE mounts only as `fuse`, without the subtype.
2. **Never enter** kernel filesystems: proc, sysfs, devtmpfs, devpts, cgroup, cgroup2, debugfs, tracefs, securityfs, pstore, bpf, binfmt_misc, selinuxfs, efivarfs, configfs, mqueue, hugetlbfs, fusectl, nsfs, rpc_pipefs, and autofs mounts that are not mounted. Also skip `/proc`, `/sys`, `/dev` and `/run` by path, as gdu does by default. ncdu's `--exclude-kernfs` does the same by filesystem magic number.
3. **Remote and FUSE mounts:** nfs, nfs4, cifs, smb3, smbfs, 9p, afs, ceph, glusterfs and every `fuse.*` type (`fuse.sshfs`, `fuse.gvfsd-fuse` under `/run/user/<uid>/gvfs`, rclone, s3fs). Show them as stubs and do not enter them unless the user asks. Reading them downloads data, as with OneDrive placeholders on Windows. `fuseblk` (ntfs-3g) is a local disk.
4. **Mounts that would count space twice:** squashfs (`/snap/*`, AppImage `/tmp/.mount_*`) is skipped, because the space is already counted in the `.snap` files under `/var/lib/snapd/snaps`. overlay (Docker `merged` folders) is skipped too. tmpfs is shown separately as memory, not disk.
5. **One mount by default.** The scan does not cross into another mount; such entries are marked, like ncdu's `>`. The delete walker never crosses a mount either. Open question: Btrfs subvolumes have their own `st_dev`, so "one filesystem" should mean one mount, not one `st_dev`, or a scan of `/` stops at `@home`.
6. **Hard links and size.** Count a file once per `(st_dev, st_ino)` when `st_nlink > 1`. Show allocated size (`st_blocks * 512`) by default, so sparse VM disks and Docker images show what they really cost; apparent size (`st_size`) is a toggle.
7. **Links and cache tags.** Use `lstat` and the entry's `d_type`, and never follow symlinks. A valid `CACHEDIR.TAG` file (an exact 43-byte signature) is a strong hint that a folder is a cache.

### Deleting safely

- `O_NOFOLLOW` only guards the last path component. The safe pattern is the one BleachBit and Rust's std use: open the trusted root with `O_DIRECTORY | O_NOFOLLOW`, then walk with `openat(dirfd, name, O_NOFOLLOW | O_DIRECTORY)`. Check `fstat` `st_dev` against the root, and remove with `unlinkat(dirfd, name, 0 or AT_REMOVEDIR)`. On Linux 5.6+, `openat2` with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_XDEV` is stronger.
- `std::fs::remove_dir_all` was fixed for CVE-2022-21658 (a check-then-delete race; Rust 1.0.0 to 1.58.0) and uses `openat`/`unlinkat` on Unix. It still descends into mounts **[docs are silent; believed to cross]**, so the project keeps its own walker with the `st_dev` check.
- Linux has no "file in use" error. The counterpart of the Windows check for running apps is `/proc/*/fd`: the process names in `closeApps`, plus open files for `/tmp` cleaning. BleachBit's `/tmp` cleaner touches only regular files owned by the user, on the same device, not open, and not on a keep list (X locks, sockets, IPC folders). ReClaim should follow the same rule.
- Linux removes a file when its folder is writable, whatever the file's own mode. `set_readonly(false)` in `clean::remove_file` does not help on Linux and should be skipped there.

## 3. User-level cleanup rules

Variables the rule engine needs: `%HOME%`, `%XDG_CACHE%`, `%XDG_CONFIG%`, `%XDG_DATA%`, `%XDG_STATE%`, each falling back to the spec default. The XDG spec says relative values must be ignored. Rules also need sandbox variants:

- **Flatpak:** `~/.var/app/<id>/{cache,config,data}` are the app's own XDG folders, so `~/.config/X` becomes `~/.var/app/<id>/config/X`.
- **Snap (strict confinement):** `HOME` is `~/snap/<name>/<rev>`. Use only the `current` link and `common`, never numbered revisions, so nothing is counted twice. Classic snaps (`code`, `android-studio`) use the real `HOME`.

Most Flatpak and Snap paths below are derived from these conventions and were not checked app by app, so those rules ship with `verified: false`.

### Browsers

| Rule | Linux path | Risk | Source |
|---|---|---|---|
| Chrome (Beta: `google-chrome-beta`, Dev: `google-chrome-unstable`) | `~/.cache/google-chrome/*/{Cache,Code Cache}` | safe | Chromium `user_data_dir.md` |
| Chromium, Brave, Edge, Vivaldi | `~/.cache/chromium`, `~/.cache/BraveSoftware/Brave-Browser`, `~/.cache/microsoft-edge`, `~/.cache/vivaldi` (same layout) | safe | Chromium doc; Brave, Edge and Vivaldi **[unverified]** |
| Opera | `~/.cache/opera/Cache` | safe | **[unverified]** |
| Service Worker CacheStorage | `~/.config/<browser>/*/Service Worker/CacheStorage` | review | Chromium doc |
| Firefox | `~/.cache/mozilla/firefox/<profile>/{cache2,startupCache}`. Profiles come from `profiles.ini` in `~/.mozilla/firefox` and, for new installs since Firefox 147, `~/.config/mozilla/firefox` | safe | Mozilla bug 259356; Firefox 147 change from secondary sources |

Chromium honours `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`. `CHROME_CONFIG_HOME` moves the profile but not the cache.

### Electron and desktop apps

Electron keeps its cache inside userData, which is `~/.config/<App>` on Linux, not in `~/.cache`. The safe subfolders are `Cache` (holds `Cache_Data`), `Code Cache`, `GPUCache`, `DawnGraphiteCache`, `DawnWebGPUCache` and `Crashpad/completed` (Electron `app.getPath` docs). This covers VS Code (`Code`), Code - Insiders, VSCodium, Cursor, Windsurf, Discord (`discord`), Slack, Signal, Element, Obsidian (`obsidian`), Postman, Teams for Linux, Heroic, LM Studio (`LM Studio`) and Claude Desktop (`Claude`, beta `.deb` since June 2026; folder name **[unverified]**). Folder names other than VS Code's are **[unverified]**.

Exceptions:

- Spotify uses CEF: `~/.cache/spotify/{Data,Storage,Browser}`.
- Telegram keeps its cache inside the session folder: only `~/.local/share/TelegramDesktop/tdata/user_data*/{cache,media_cache}` may be cleaned, never the rest of `tdata` (`review`).
- Signal: never `sql/` or `attachments.noindex`.
- Zoom: `~/.zoom/logs`.

### Developer caches

| Tool | Linux path (override) | Risk | Official command |
|---|---|---|---|
| npm | `~/.npm/_cacache`, `~/.npm/_npx` (`npm_config_cache`); not XDG | safe | `npm cache clean --force` |
| pnpm | store `~/.local/share/pnpm/store` (`storeDir`), cache `~/.cache/pnpm` | review / safe | `pnpm store prune` |
| yarn | classic `~/.cache/yarn` (`YARN_CACHE_FOLDER`), berry `~/.yarn/berry/cache` | safe | `yarn cache clean` |
| bun, deno | `~/.bun/install/cache`, `~/.cache/deno` (`DENO_DIR`) | review / safe | `bun pm cache rm`, `deno clean` |
| pip, uv, poetry, pipx | `~/.cache/pip`, `~/.cache/uv`, `~/.cache/pypoetry/{cache,artifacts}`, `~/.cache/pipx` (venvs in `~/.local/share/pipx` are not a cache) | safe | `pip cache purge`, `uv cache clean`, `poetry cache clear --all` |
| conda | `~/{miniconda3,anaconda3,miniforge3}/pkgs`, `~/.conda/pkgs` | review | `conda clean --all` |
| cargo | `~/.cargo/registry/{cache,src}`, `~/.cargo/git/checkouts` (`CARGO_HOME`); not XDG | safe (`git/db` review) | none |
| go | build `~/.cache/go-build` (`GOCACHE`), modules `~/go/pkg/mod` (read-only files) | safe / review | `go clean -cache`, `go clean -modcache` |
| gradle, maven | `~/.gradle/{caches,wrapper/dists,daemon}`, `~/.m2/repository` | review / safe / safe, caution | none |
| NuGet | `~/.nuget/packages`, `~/.local/share/NuGet/v3-cache` | review / safe | `dotnet nuget locals all --clear` |
| composer, pub | `$XDG_CACHE_HOME/composer` (`COMPOSER_CACHE_DIR`), `~/.pub-cache` | safe / review | `composer clear-cache`, `dart pub cache clean` |
| Android | `~/Android/Sdk/{system-images,.temp,ndk}`, AVDs `~/.android/avd` | review / caution | none |
| JetBrains | `~/.cache/JetBrains/<product><version>` (logs in `log/`); Android Studio `~/.cache/Google/AndroidStudio*` | review / safe | none |
| Browser test tools | `~/.cache/ms-playwright`, `~/.cache/puppeteer`, `~/.cache/Cypress` | review | `cypress cache clear` |
| Build caches | `~/.cache/electron`, `~/.cache/electron-builder`, `~/.cache/node-gyp`, `~/.cache/ccache`, `~/.cache/sccache`, `~/.cache/bazel` | safe (Bazel review) | `ccache -C`, `bazel clean --expunge` |
| VMs and containers | Docker Desktop `~/.docker/desktop/vms/0/data/Docker.raw`, Podman `~/.local/share/containers/storage`, Vagrant `~/.vagrant.d/{boxes,tmp}` | report / review | `docker system prune`, `podman system prune`, `vagrant box prune` |

The project-artifact rules (`rules/artifacts.json`, 42 rules) apply unchanged.

### AI models

| Item | Linux path (override) | Risk |
|---|---|---|
| Hugging Face | `~/.cache/huggingface/{hub,xet,assets,datasets}` (`HF_HOME`, `HF_HUB_CACHE`, `HF_XET_CACHE`, `HF_DATASETS_CACHE`) | review / safe |
| Ollama, user install | `~/.ollama/models` (`OLLAMA_MODELS`) | review, remove with `ollama rm` |
| Ollama, systemd service | `/usr/share/ollama/.ollama/models`, owned by the `ollama` user | report only |
| LM Studio | `~/.lmstudio/models`, legacy `~/.cache/lm-studio`, redirect file `~/.lmstudio-home-pointer` | review |
| torch hub, whisper | `~/.cache/torch/hub` (`TORCH_HOME`), `~/.cache/whisper` | review |
| ComfyUI | No fixed folder (a git clone; comfy-cli uses `~/comfy/ComfyUI` **[unverified]**). Found through markers, like the Windows page | caution |

### Desktop

| Item | Path | Risk / how |
|---|---|---|
| Thumbnails | `~/.cache/thumbnails/{normal,large,x-large,xx-large,fail}`, legacy `~/.thumbnails` | safe, rebuilt on demand |
| Trash | `~/.local/share/Trash`, `$mount/.Trash-$uid` | user data: spec-aware empty, after confirmation (`gio trash --empty`) |
| Shader caches | Mesa `~/.cache/mesa_shader_cache*` (`MESA_SHADER_CACHE_DIR`, 1 GB cap); NVIDIA `~/.cache/nvidia/GLCache`, legacy `~/.nv/GLCache`; AMDVLK `~/.cache/AMD/VkCache` **[unverified]** | safe |
| Search indexes | GNOME LocalSearch `~/.cache/tracker3/files`, KDE Baloo `~/.local/share/baloo/index` | review; `localsearch reset`, `balooctl6 purge` **[unverified]** |
| fontconfig | `~/.cache/fontconfig` | safe, `fc-cache -f` |
| `~/.xsession-errors` | X session log | safe; truncate instead of deleting while the session runs |

### Games

- **Steam root:** `~/.local/share/Steam`. `~/.steam/steam` and `~/.steam/root` are links to it, and older Debian installs use `~/.steam/debian-installation`. Flatpak: `~/.var/app/com.valvesoftware.Steam/.local/share/Steam`. Snap: `~/snap/steam/common/.local/share/Steam`. Resolve the links and deduplicate.
- **Libraries:** `steamapps/libraryfolders.vdf`, which the existing VDF parser already reads. Shader caches (`steamapps/shadercache/<appid>`), `steamapps/temp`, logs and htmlcache work as on Windows.
- **Proton prefixes** (`steamapps/compatdata/<appid>/pfx`): **caution, report only**. They hold save games outside Steam Cloud (`drive_c/users/steamuser/{Documents,AppData,Saved Games}`). At most, list the prefixes of uninstalled games. Lutris, Heroic and `~/.wine` prefixes are treated the same way. Their caches (`~/.cache/lutris` subfolders, Heroic's Electron cache, `~/.cache/wine` installers) are safe.

## 4. System areas: report rules with the official command

None of these is deleted as files. Each is measured, explained, and offered with the tool that owns it.

| Area | Where | Measure without root | Official command (root) | Note |
|---|---|---|---|---|
| apt | `/var/cache/apt/archives/*.deb` | yes (`partial/` is not readable) | `apt-get clean`, `apt-get autoclean`, `apt-get autoremove` (dry run with `-s`, no root) | `autoremove` can remove packages the user wanted |
| dnf | `/var/cache/libdnf5` (dnf4: `/var/cache/dnf`) | yes **[unverified]** | `dnf clean all` | `keepcache` is off, so this is mostly metadata; old kernels follow `installonly_limit` (default 3) |
| pacman | `/var/cache/pacman/pkg` (keeps every version) | yes | `paccache -r` (pacman-contrib, keeps 3), `paccache -ruk0` | needed to downgrade on Arch |
| zypper | `/var/cache/zypp/packages` | yes | `zypper clean --all`; kernels: `zypper purge-kernels` | |
| journald | `/var/log/journal/<machine-id>` | `journalctl --disk-usage` is full only for members of `systemd-journal`, `adm` or `wheel` | `journalctl --rotate --vacuum-size=…` or `--vacuum-time=…` | Removes only archived files. Default cap: 10% of the filesystem, at most 4 GB |
| Core dumps | `/var/lib/systemd/coredump` | `coredumpctl list` (own dumps) | none; systemd ages them out (2 weeks, `MaxUse`) | Ubuntu Apport `/var/crash/*.crash`: own files can go without root |
| Snap | disabled revisions in `/var/lib/snapd/snaps`, `/var/lib/snapd/cache`, snapshots in `/var/lib/snapd/snapshots` | `snap list --all` plus file sizes | `snap remove <name> --revision=<rev>`, `snap set system refresh.retain=2` | Snapshots are backups of removed apps' data (review). The cache is hard-linked, so it may free nothing **[unverified]** |
| Flatpak | `/var/lib/flatpak`, `~/.local/share/flatpak` | yes; count each inode once (OSTree hard links) | `flatpak uninstall --unused`, `flatpak repair` | System installs ask through polkit, no sudo. `--delete-data` removes user data: never by default |
| Docker | `/var/lib/docker` (root only) | `docker system df` (the `docker` group is root-equivalent) | `docker system prune` (`-a` adds images, `--volumes` adds data) | Report only. Rootless Docker and Podman: same commands as the user. Plain `du` is wrong under user namespaces |
| `/tmp`, `/var/tmp` | tmpfs on Fedora, Arch and Debian 13; often on disk on Ubuntu **[unverified]** | own files only | `systemd-tmpfiles --clean` (`/tmp` 10 days, `/var/tmp` 30 days) | Never sockets (`.X11-unix`, `ssh-*`, `systemd-private-*`), AppImage mounts (`.mount_*`) or open files |
| Old logs | `/var/log/**/*.gz`, `*.1` | partly (`adm` group) | none; logrotate decides | report only |

Not worth a rule: the locate database, the man-db cache and the system fontconfig cache. They are small and rebuilt by their own tools.

## 5. Hidden space on Linux

What the reconciliation page can list, as the Linux side of `hidden.rs`:

1. **Reserved blocks** (ext4 keeps 5% for root by default): `(f_bfree - f_bavail) * f_frsize` from `statvfs`, no root needed. Changing it (`tune2fs -m`) is report only.
2. **Filesystem metadata and journal**: ext4 inode tables and journal, Btrfs metadata chunks. Shown as an estimate in the remainder.
3. **Snapshots**: Btrfs (Snapper `/.snapshots`, Timeshift), ZFS, LVM. On Btrfs, `du` counts shared extents once per snapshot while `df` counts each block once. `btrfs filesystem du` gives total, exclusive and shared sizes (whether it needs root **[unverified]**). Deleting goes through `snapper delete`, `timeshift --delete` or `zfs destroy`, all as root.
4. **Swap**: `/proc/swaps` is readable by everyone. Linux has no hiberfil; hibernation uses swap, and zram takes no disk.
5. **Deleted but still open files**: `/proc/<pid>/fd/*` links ending in `(deleted)`. Only the user's own processes are visible without root. The space returns when the process exits. This is a common surprise on Linux and worth its own line.
6. **Files hidden under mount points**: for example old `/tmp` contents after Debian 13 moved `/tmp` to tmpfs. Detecting them needs a root bind mount, so the page only explains this case.
7. **Sparse VM disks**: qcow2 and raw images, where allocated and apparent size differ (`qemu-img info`; `virt-sparsify` reclaims). Common places: `~/.local/share/gnome-boxes/images`, `/var/lib/libvirt/images` **[unverified]**.
8. **Outside the filesystem**: free space in an LVM volume group or thin pool never shows in `df` (`vgs`, root).
9. **Unreadable areas**: `/var/lib/docker`, `/var/lib/snapd`, other users' homes. Shown as "unreadable without root", not hidden.

## 6. Elevation

- **The pkexec rules:**
  - `pkexec` checks a polkit action. The generic one is `org.freedesktop.policykit.exec`, or an app ships its own `.policy` file with `org.freedesktop.policykit.exec.path` naming one exact binary.
  - pkexec clears `DISPLAY`, so GUI programs should not run through it.
  - It needs a running polkit agent.
- **AppImage:** the app runs from a per-user FUSE mount (`/tmp/.mount_XXXX`). Root cannot enter it, and its path changes on every start, so neither `pkexec` on the app itself nor a fixed `exec.path` works. CPU-X documents the same failure.
- **How other tools do it:**
  - Stacer runs `pkexec rm -rf <paths>`: paths cross into root, and every command asks for the password again. To avoid.
  - BleachBit offers a whole GUI running as root. To avoid.
  - GNOME Disks never runs as root: it calls udisks2 over D-Bus, and udisks2 checks fine-grained polkit actions. snapd, flatpak-system-helper, PackageKit and journald work the same way. This is the model to follow.
- **Proposal for ReClaim:**
  1. Phase 1: report rules show the size and a ready-to-copy command, and nothing runs as root.
  2. Later: run only system-installed official binaries through `pkexec` (`/usr/bin/apt-get clean`, `/usr/bin/journalctl --vacuum-size=…`), with a fixed argument list per rule id. The existing rule that an elevated step receives ids, never paths, stays.
  3. PackageKit (`pkcon`) may give one cross-distro path for package cleanup **[unverified that it covers cleaning]**.

## Open questions

1. Is "one filesystem" one mount or one `st_dev`? Btrfs subvolumes make the difference visible.
2. Should tmpfs `/tmp` count as memory rather than disk? (Proposed: yes, shown separately.)
3. Does `/var/lib/snapd/cache` hard-link `snaps/`? If so, deleting it frees nothing.
4. Should `/nix/store` and the Fedora Atomic trees go on list A, with `nix-collect-garbage` as a report rule?
5. Ubuntu's `/tmp` default, Apport and ABRT status, Snap and most Flatpak paths need a check on real VMs (Ubuntu 24.04, Fedora 43, Arch) before rules leave `verified: false`.

## Sources

Specs and man pages:
- [UAPI.9 file hierarchy](https://uapi-group.org/specifications/specs/linux_file_system_hierarchy/), [file-hierarchy(7)](https://www.freedesktop.org/software/systemd/man/latest/file-hierarchy.html), [hier(7)](https://man7.org/linux/man-pages/man7/hier.7.html)
- [XDG Base Directory](https://specifications.freedesktop.org/basedir/latest/), [user-dirs.dirs(5)](https://man.archlinux.org/man/user-dirs.dirs.5), [Trash spec](https://specifications.freedesktop.org/trash/latest/), [Thumbnail spec](https://specifications.freedesktop.org/thumbnail/latest/directory.html), [CACHEDIR.TAG](https://bford.info/cachedir/)
- [proc_pid_mountinfo(5)](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html), [statfs(2)](https://man7.org/linux/man-pages/man2/statfs.2.html), [inode(7)](https://man7.org/linux/man-pages/man7/inode.7.html), [open(2)](https://man7.org/linux/man-pages/man2/open.2.html), [openat2(2)](https://man7.org/linux/man-pages/man2/openat2.2.html), [unlinkat(2)](https://man7.org/linux/man-pages/man2/unlinkat.2.html), [rm(1)](https://man7.org/linux/man-pages/man1/rm.1.html)
- [Rust remove_dir_all](https://doc.rust-lang.org/std/fs/fn.remove_dir_all.html), [CVE-2022-21658](https://blog.rust-lang.org/2022/01/20/cve-2022-21658/)
- [journalctl(1)](https://www.freedesktop.org/software/systemd/man/latest/journalctl.html), [journald.conf(5)](https://www.freedesktop.org/software/systemd/man/latest/journald.conf.html), [coredump.conf(5)](https://www.freedesktop.org/software/systemd/man/latest/coredump.conf.html), systemd [tmp.conf](https://raw.githubusercontent.com/systemd/systemd/main/tmpfiles.d/tmp.conf) and [systemd.conf.in](https://raw.githubusercontent.com/systemd/systemd/main/tmpfiles.d/systemd.conf.in)
- [apt-get(8)](https://manpages.ubuntu.com/manpages/noble/man8/apt-get.8.html), [dnf5.conf(5)](https://dnf5.readthedocs.io/en/latest/dnf5.conf.5.html), [SUSE multikernel](https://documentation.suse.com/es-es/sles/15-SP5/html/SLES-all/cha-tuning-multikernel.html), [Debian 13 release notes](https://www.debian.org/releases/trixie/release-notes/issues.en.html)
- [Snap data locations](https://snapcraft.io/docs/reference/administration/data-locations/), [Snap environment variables](https://snapcraft.io/docs/environment-variables), [Snap updates](https://snapcraft.io/docs/how-to-guides/manage-snaps/manage-updates/), [Flatpak conventions](https://docs.flatpak.org/en/latest/conventions.html), [Flatpak commands](https://docs.flatpak.org/en/latest/flatpak-command-reference.html)
- [Docker post-install](https://docs.docker.com/engine/install/linux-postinstall/), [Docker rootless](https://docs.docker.com/engine/security/rootless/), [Docker Desktop for Linux FAQ](https://docs.docker.com/desktop/troubleshoot-and-support/faqs/linuxfaqs/), [btrfs filesystem](https://btrfs.readthedocs.io/en/latest/btrfs-filesystem.html)
- [pkexec(1)](https://www.freedesktop.org/software/polkit/docs/latest/pkexec.1.html), [udisks polkit actions](https://storaged.org/udisks/docs/udisks-polkit-actions.html)

Tools (GPL ones read for ideas only):
- [BleachBit](https://github.com/bleachbit/bleachbit) (`Cleaner.py`, `FileUtilities.py`), [safe-rm](https://git.launchpad.net/safe-rm/plain/src/main.rs), [Stacer](https://github.com/oguzhaninan/Stacer)
- [Czkawka](https://github.com/qarmin/czkawka/blob/master/czkawka_core/src/common/items.rs), [trash-cli](https://github.com/andreafrancia/trash-cli), [rmlint](https://rmlint.readthedocs.io/en/latest/rmlint.1.html), [KDE Sweeper](https://apps.kde.org/sweeper/)
- [gdu](https://github.com/dundee/gdu/blob/master/pkg/device/dev_linux.go), [ncdu](https://dev.yorhel.nl/ncdu/man), [dua-cli](https://github.com/Byron/dua-cli), [dust](https://github.com/bootandy/dust), [filelight](https://github.com/KDE/filelight/blob/master/src/filelightsettings.kcfg), [QDirStat](https://github.com/shundhammer/qdirstat/blob/master/src/MountPoints.cpp), [baobab](https://github.com/GNOME/baobab/blob/master/data/org.gnome.baobab.gschema.xml)
- [CPU-X AppImage notes](https://github.com/TheTumultuousUnicornOfDarkness/CPU-X/wiki/appimage)

Cache locations:
- Browsers and Electron: [Chromium user data dir](https://chromium.googlesource.com/chromium/src/+/main/docs/user_data_dir.md), [Electron app.getPath](https://www.electronjs.org/docs/latest/api/app#appgetpathname), [Mozilla bug 259356](https://bugzilla.mozilla.org/show_bug.cgi?id=259356)
- JavaScript: [npm folders](https://docs.npmjs.com/cli/configuring-npm/folders), [pnpm settings](https://pnpm.io/settings#storedir), [yarn](https://yarnpkg.com/configuration/yarnrc), [bun cache](https://bun.sh/docs/install/cache), [deno env](https://docs.deno.com/runtime/reference/env_variables/)
- Python: [pip cache](https://pip.pypa.io/en/stable/topics/caching/), [uv cache](https://docs.astral.sh/uv/concepts/cache/), [poetry](https://python-poetry.org/docs/configuration/#cache-dir), [pipx paths](https://github.com/pypa/pipx/blob/main/src/pipx/paths.py), [conda clean](https://docs.conda.io/projects/conda/en/latest/commands/clean.html)
- Rust and Go: [cargo home](https://doc.rust-lang.org/cargo/guide/cargo-home.html), [go build cache](https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching), [go module cache](https://go.dev/ref/mod#module-cache)
- JVM, .NET, PHP, Dart, Android: [gradle layout](https://docs.gradle.org/current/userguide/directory_layout.html), [NuGet folders](https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders), [composer cache-dir](https://getcomposer.org/doc/06-config.md#cache-dir), [pub env](https://dart.dev/tools/pub/environment-variables), [Android variables](https://developer.android.com/tools/variables), [JetBrains directories](https://www.jetbrains.com/help/idea/directories-used-by-the-ide-to-store-settings-caches-plugins-and-logs.html)
- Test and build tools: [Playwright](https://playwright.dev/docs/browsers), [Puppeteer](https://pptr.dev/guides/configuration), [Cypress](https://docs.cypress.io/app/references/advanced-installation), [ccache](https://ccache.dev/manual/latest.html#config_cache_dir), [Bazel](https://bazel.build/remote/output-directories)
- AI: [Hugging Face env](https://huggingface.co/docs/huggingface_hub/package_reference/environment_variables), [Ollama FAQ](https://docs.ollama.com/faq), [torch hub](https://pytorch.org/docs/stable/hub.html)
- Graphics: [Mesa env vars](https://docs.mesa3d.org/envvars.html), [NVIDIA GL env vars](https://download.nvidia.com/XFree86/Linux-x86_64/570.86.16/README/openglenvvariables.html)
- Games: [Steam Flatpak wiki](https://github.com/flathub/com.valvesoftware.Steam/wiki), [Proton FAQ](https://github.com/ValveSoftware/Proton/wiki/Proton-FAQ), [Heroic paths](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher/blob/main/src/backend/constants/paths.ts), [Lutris](https://github.com/lutris/lutris), [Wine FAQ](https://gitlab.winehq.org/wine/wine/-/wikis/FAQ)
