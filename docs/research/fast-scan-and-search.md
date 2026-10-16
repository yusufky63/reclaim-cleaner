# Research 2: WizTree-speed scanning and Everything-style search for ReClaim Cleaner

Research date: 2026-10-16. Scope: how to make ReClaim's drive scan as fast as WizTree, add an Everything-like instant search with file-type filters, and keep the app free of an always-running service.

Labels used below, as in `docs/research.md`:

- **(verified)**: checked in a primary source (Microsoft Learn, crate docs, project source) or measured on the development machine during this research.
- **(secondary)**: reported by a third party (forum post, PR description, README benchmark) and not reproduced here.
- **(unverified)**: my estimate or inference. Each needs a measurement before it drives a decision.

Licensing note: WinDirStat is GPL-3.0 and UltraFastFileSearch is MPL-2.0. They are cited as design references only. ReClaim is MIT, so no code is copied from them. The NTFS on-disk structures themselves are public facts.

---

## 0. Summary

1. **The 108 s figure is mostly not the file system.** It comes from a debug build of a walker that uses `std::fs::read_dir`. On Windows that calls `FindFirstFileExW(FindExInfoBasic)` without `FIND_FIRST_EX_LARGE_FETCH` (verified, Rust std source). The walker also allocates several strings per file. Measure a release build first, then move the walker to `NtQueryDirectoryFile` with a large buffer, as WinDirStat does (verified, WinDirStat source).
2. **WizTree speed needs a raw `$MFT` read.** This means elevation. `FSCTL_ENUM_USN_DATA` is simpler but returns no sizes, so a disk analyzer cannot use it alone. A raw read gives names, parents, logical and allocated sizes, timestamps, attributes, reparse tags and hard links in one sequential pass. Published numbers: 5.3M files in about 4.9 s against 7.1 s for a walker on NVMe, and 870k files in 2.2 s against 8.3 s (secondary). On this machine C: has an 8.99 GiB `$MFT` with 9.43M record slots on NVMe (verified), so a raw read should take roughly 4 to 7 s (unverified estimate).
3. **No service is needed for live updates.** On this machine, a non-elevated process with a handle to `C:\` successfully called `FSCTL_QUERY_USN_JOURNAL`, `FSCTL_GET_NTFS_VOLUME_DATA` and `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` (verified). The unprivileged journal read returns file and parent reference numbers and reason flags, with names stripped (`FileNameLength = 0`, verified). That is enough to keep an index current while the app is open. Changed entries are re-stated by their known path, and new names come from re-listing the parent folder.
4. **The journal is short.** The C: journal here is 32 MB, and at the time of measurement it covered only 0.8 hours of activity (381,643 records, read in 79 ms without admin, verified). "Catch up from the journal when the app opens" therefore works only for short gaps. Plan for a fast full rebuild: MFT when the user approves UAC, the fast walker otherwise. Show the cached index immediately while it rebuilds.
5. **Recommended crates.** Write a small MFT reader inside `reclaim-core` on top of the `windows` crate (0.62.2, already a dependency). Use `ntfs-reader` 0.6.0 (MIT OR Apache-2.0) only as a test oracle. It exposes no allocated size, `Mft::new` holds about 1 KiB per file (around 9 GiB here), and `MftScan` reads `$MFT` twice. For search: `memchr`, `regex`, `globset`, `lz4_flex`. For the UI: `@tanstack/react-virtual` 3.14.20.
6. **Index memory.** Expect about 45 to 60 MB per million entries for a columnar index with sizes and dates (unverified estimate). That is in line with Everything's documented 100 MB per million files. For C: here, that is 300 to 400 MB. Make "instant search" opt-in.

---

## 1. Baseline on the development machine

### 1.1 Volumes (verified, measured without admin)

| Item | C: | D: |
|---|---|---|
| File system | NTFS | NTFS |
| Disk | NVMe SSD | SATA SSD (the one non-C: disk) |
| Size / free | 930.5 GiB / 140.0 GiB | 953.9 GiB / 68.1 GiB |
| Cluster / file record | 4096 B / 1024 B | not measured |
| `$MFT` valid data | 9,655,812,096 B (8.99 GiB) = 9,429,504 record slots | not measured |
| USN journal | ID 0x01d861d3d3352ef2, max 32 MB, allocation delta 8 MB, record versions 2 to 4 | not measured |
| Journal coverage at measurement | 381,643 records spanning 0.8 h (22:38Z to 23:24Z), read in 79 ms | not measured |
| Files (from the current walker) | about 5.8M files, 108 s debug build | not measured |

Cluster, record size and `$MFT` length came from `FSCTL_GET_NTFS_VOLUME_DATA` on a `C:\` directory handle. The journal values came from `fsutil usn queryjournal c:` and an unprivileged read. Probe scripts are in this scratchpad folder: `usnprobe.ps1`, `usnspan.ps1`, `ntfsvol.ps1`. They print counts and timestamps only, never file names.

### 1.2 What a non-elevated process can do (verified on Windows 11 build 26200)

| Handle | FSCTL | Result |
|---|---|---|
| `\\.\C:` with access 0 or `FILE_READ_ATTRIBUTES` | open | ok |
| `\\.\C:` with `GENERIC_READ` | open | error 5 (access denied) |
| `\\.\C:` with access 0 | `FSCTL_QUERY_USN_JOURNAL` | error 1 (invalid function) |
| `C:\` with `FILE_FLAG_BACKUP_SEMANTICS` + `FILE_READ_ATTRIBUTES` | `FSCTL_QUERY_USN_JOURNAL` | ok |
| same | `FSCTL_GET_NTFS_VOLUME_DATA` | ok (gives `$MFT` size without admin, so a cost check is possible) |
| same | `FSCTL_READ_USN_JOURNAL` | error 5 |
| same | `FSCTL_ENUM_USN_DATA` | error 5 |
| same | `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` (0x903AB) | ok. V2 records have FRN and parent FRN, `FileNameLength = 0`. A start USN in the middle of a record returns error 87. Use 0, `FirstUsn` or a USN taken from a previous read |

`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` is defined in the `windows` crate 0.62.2 as `590763` (verified). Its Microsoft Learn page returned 404 during this research, so its exact contract is undocumented here. Two things are unverified: whether records for files the user cannot access are filtered out, and the minimum Windows build. Treat it as an optimization with a full-rescan fallback.

### 1.3 Why the current walker is slow

These come from reading `crates/core/src/walk.rs` and `explorer.rs`.

- `std::fs::read_dir` calls `FindFirstFileExW(FindExInfoBasic, …, 0)`. It deliberately does not pass `FIND_FIRST_EX_LARGE_FETCH` (verified, `library/std/src/sys/fs/windows.rs`). Each `FindNextFileW` copies into a `WIN32_FIND_DATAW`.
- Per file, `scan_raw` does `entry.file_name().to_string_lossy().into_owned()`, `rsplit_once('.')`, `to_lowercase()` (another allocation) and a `HashMap<String, …>` insert. Per folder it does `entry.path()` (a `PathBuf` allocation) and a global `Mutex` merge of extension stats.
- The tree is built twice: a `RawDir` tree, then `flatten` into `Node`s that each own a `String` and a `Vec<u32>`.
- A debug build disables inlining and optimization. 108 s for 5.8M files is about 18.6 µs per file. A release build alone should be several times faster (unverified, Phase 0 measures it).

For reference, a release walker in another Rust project took about 7.1 s for 5.3M files warm on NVMe (secondary, disk-cleaner PR #89; walker implementation not stated).

---

## 2. How the fast tools get their speed

### 2.1 WizTree (proprietary)

- "When scanning NTFS formatted drives, WizTree reads the hard drive's Master File Table (MFT) directly from the disk" (verified, diskanalyzer.com).
- It counts hard links once, and its allocated totals match what Windows reports (verified, same page).
- MFT mode needs administrator rights. Without them it uses a slower scan, and versions 4.21 and 4.28 sped up that non-admin scan (secondary: forum posts and release-note coverage).
- On the rsdirstat test machine, WizTree 4.33 scanned C: (705 GB, 498k folders) in 7.27 s warm with CSV export. rsdirstat's own MFT reader took 1.97 s (secondary, rsdirstat PR #6).

### 2.2 WinDirStat 2.5+ (GPL-3.0; source read as a design reference)

MFT finder (`windirstat/FinderNtfs.cpp`, verified by reading the source):

1. Open `\\.\C:` with `FILE_READ_DATA | FILE_READ_ATTRIBUTES`, `FILE_FLAG_NO_BUFFERING | FILE_FLAG_OVERLAPPED`.
2. `FSCTL_GET_NTFS_VOLUME_DATA` gives the cluster size and the file record size.
3. Open `C:\$MFT::$DATA` with only `FILE_READ_ATTRIBUTES` and call `FSCTL_GET_RETRIEVAL_POINTERS`. This returns the `$MFT` extents without parsing record 0's runlist and without reading the locked file through its own handle.
4. Read every extent in parallel (`std::execution::par`) in 4 MiB page-aligned overlapped reads.
5. For each 1 KiB record: check the `FILE` signature and the update sequence array bounds, apply fixups, and skip records that are not in use.
6. Attributes:
   - `$STANDARD_INFORMATION` gives mtime and attributes.
   - `$FILE_NAME` gives the name and parent. Skip DOS-only (8.3) names and `.`/`..`.
   - Unnamed `$DATA` with `LowestVcn == 0` gives sizes. Logical size is `FileSize`. Physical size is the compressed-size field when the attribute is compressed or sparse, otherwise `AllocatedLength`. A resident stream counts as `(len + 7) & ~7`.
   - The named stream `WofCompressedData` gives the physical size of WOF/CompactOS files.
   - `$REPARSE_POINT` gives the tag. WOF counts as compressed, and junctions are detected.
7. Extension records (`BaseFileRecordNumber != 0`) are merged into their base record after the pass. This avoids parsing `$ATTRIBUTE_LIST` at all.
8. The result is a flat record table indexed by record number, a `parent -> children` map, and per-extent name arenas.

Non-admin walker (`FinderBasic.cpp`, verified):

- `NtQueryDirectoryFile` with `FileIdFullDirectoryInformation`, falling back to `FileFullDirectoryInformation`.
- 4 MiB thread-local buffer for local volumes, 64 KiB for remote ones ("Larger buffers fail on older network redirectors").
- Directories opened with `FILE_OPEN_FOR_BACKUP_INTENT`. `IO_REPARSE_TAG_MOUNT_POINT` is told apart from junctions by reading the reparse buffer.

### 2.3 TreeSize (proprietary)

Uses the MFT on local NTFS and falls back to a threaded walk elsewhere (secondary, already in `docs/research.md`).

### 2.4 Everything (voidtools, proprietary)

- "NTFS indexing requires the Everything service or running 'Everything' as administrator" (verified, FAQ).
- Index time: about 250,000 files in about 5 s, and 1,000,000 files in about 1 minute (verified, FAQ).
- Memory: about 35 MB RAM and under 14 MB disk for 250,000 files; about 100 MB RAM and 45 MB disk for 1,000,000 files (verified, FAQ). With optimized settings the developer gives about 50 MB per million (secondary, forum).
- Version 1.4 indexes size and date and keeps "fast sort" data for them by default (verified, FAQ). Other properties are read from disk when sorted, which is slow (verified, indexes docs).
- Internals, from the developer's forum reply (secondary):
  - The database is built from the MFT and kept current with the USN journal.
  - It is "basically a list of all the file names (in UTF-8)", plus size, date modified and parent pointers.
  - It has indexes for name, path, size and date.
  - Search is "an optimized multi-threaded strstr search on every single filename", with queries compiled to byte code.
  - The database is held in memory, saved on exit and loaded at start.
- FAT, exFAT and network folders are indexed by walking, with rescans (verified, FAQ).
- Privacy trade-off: users report that one account's index shows another account's files (secondary, forum). ReClaim must address this (section 7).

### 2.5 Recent open-source data points (all secondary, self-reported, different hardware)

| Project | Method | Data set | Result |
|---|---|---|---|
| rsdirstat PR #6 (Rust) | Custom raw `$MFT` read, 4 MiB pieces, up to 4 readers | C:, 870k files, NVMe | 2.2 s warm against 8.3 s walk; 3.2 s cold against 10.6 s. Full C: (705 GB) 1.97 s warm, 2.11 s cold, against 8.22 s / 10.45 s walking |
| disk-cleaner PR #89 (Rust) | Raw `$MFT` read | C:, 5.3M files | 4.85 to 4.95 s warm against 7.09 to 7.12 s walker. 30% faster on NVMe, but 3.0x and 5.9x slower than the walker on two SATA volumes |
| Tachyon (C#, MIT) | `FSCTL_ENUM_USN_DATA`, 256 KB buffer | 3.4M files, 4 drives | 15.1 s (about 226k files/s); USN polled every 500 ms |
| UltraFastFileSearch (Rust, MPL-2.0) | Raw MFT over IOCP with `$MFT` bitmap skip ("eliminates 40–55% of I/O") | 25.9M records, 7 drives | 68.5 s cold build; 5.7 s warm restart from cache |
| dirstats PR #10 | MFT reader | 100k generated files (CI) | 229 ms cold / 260 ms warm against walker 638 ms cold / 89 ms warm |
| 2015 blog mirror | `FSCTL_ENUM_USN_DATA` | 700k files | 21 s cold |

What these show:

- MFT cost scales with `$MFT` bytes divided by disk throughput. Walker cost scales with file count and the metadata cache.
- The MFT path wins cold and on NVMe. Warm runs on SATA can lose to a cached walker (disk-cleaner).
- The MFT path matches a walker "byte for byte" once the sparse/compressed size field is right (disk-cleaner). Per-folder numbers still differ for hard links, because the two scanners pick a different owning folder. One folder differed by 11.2 GB (disk-cleaner, rsdirstat).

---

## 3. Comparison of approaches

| Approach | Admin | Data returned | Sizes | Speed evidence | Accuracy notes | Complexity | File systems |
|---|---|---|---|---|---|---|---|
| (a) `FSCTL_ENUM_USN_DATA` on `\\.\C:` | Yes (verified: error 5 without) | FRN, parent FRN, name, attributes, USN, timestamp (USN_RECORD_V2/V3) | **No** | 226k files/s (Tachyon), 700k in 21 s cold (2015) (both secondary) | One record per file. Hard links come back as a single name (the record's name), so a name index is incomplete for hard links (unverified) | Low (about 150 lines) | NTFS. ReFS journal reads work, but enumeration depends on the build (usn-journal-rs README) |
| (b) Raw `$MFT` read via volume handle | Yes (open `\\.\C:` with read access) | Everything in the record: all names and parents, `$STANDARD_INFORMATION`, `$DATA` logical, allocated and compressed sizes, ADS, reparse tags, link count | **Yes**, logical and allocated | 5.3M files about 4.9 s (NVMe), 870k files 2.2 s (secondary). Here: 8.99 GiB `$MFT` at about 1.5 to 3 GB/s is 3 to 6 s of I/O (unverified) | Exact once sparse/compressed/WOF handled; matches walker totals (secondary) | Medium-high (parser plus edge cases, about 1,000 to 1,500 lines with tests) | NTFS only |
| (c) `FSCTL_QUERY_FILE_LAYOUT` | Probably yes: it takes a volume handle, and the docs list "STATUS_ACCESS_DENIED: the caller cannot access the file system volume" (unverified without admin) | `FILE_LAYOUT_ENTRY` with FRN and attributes; a chain of `FILE_LAYOUT_NAME_ENTRY` with parent FRN per name (one per hard link, unverified); `STREAM_LAYOUT_ENTRY` with `AllocationSize` and `EndOfFile`; extra info (times, USN, security id) | **Yes**, through the NTFS driver, no on-disk parsing | **Unknown**; no public benchmark found | Driver-maintained, so no fixups, attribute lists or WOF parsing on our side. WOF and compressed semantics of `AllocationSize` are unverified | Medium (variable-length entry chains) | NTFS only. Docs say "This code is not supported by ReFS". Windows 8 or later; file-reference filter ranges from Windows 10 1809 |
| (d) `NtQueryDirectoryFile` / `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)` walker | **No** | Per entry: name, `EndOfFile`, `AllocationSize`, attributes, reparse tag, file ID (64-bit or 128-bit), times | **Yes** | WinDirStat's non-admin path. NtQueryDirectoryFile about 40% faster than FindFirst/FindNext (secondary, anecdotal). For ReClaim, about 8 to 20 s for 5.8M files warm (unverified) | Same view as Explorer; ACL-blocked folders are invisible without admin | Low-medium | Any file system, including network (use a 64 KiB buffer there) |
| (e) Current `std::fs::read_dir` walker | No | Name and `WIN32_FIND_DATAW` | Logical only | 108 s debug for 5.8M files | Counts hard links once per name (double counting) | Exists | Any |
| (f) `FindFirstFileExW` + `FIND_FIRST_EX_LARGE_FETCH` | No | `WIN32_FIND_DATAW` | Logical only | About 2x on an HDD, 7 threads (secondary). Small gain on SSD (secondary) | As (e) | Low | Any |

Recommendation:

- Use **(b)** for elevated scans of local NTFS volumes.
- Use **(d)** as the universal fallback and as the non-admin default.
- Run a 1 to 2 day **(c)** spike in Phase 2. If it reaches (b)'s speed it removes the riskiest code (the record parser). Keep (b) if it does not.
- Use (a) only if a name-only index is ever wanted without sizes. That is not the case for ReClaim.

---

## 4. MFT reader pitfalls and how to handle them

| Pitfall | What happens | Handling |
|---|---|---|
| Locked `$MFT` | `C:\$MFT` cannot be opened for reading | Get extents with `FSCTL_GET_RETRIEVAL_POINTERS` on `C:\$MFT::$DATA` opened with `FILE_READ_ATTRIBUTES` (WinDirStat, verified). Read those LCN ranges through the volume handle |
| Raw volume I/O rules | Reads must be sector-aligned in offset, length and buffer | `FILE_FLAG_NO_BUFFERING` + `FILE_FLAG_OVERLAPPED`, 4 MiB buffers aligned to 4 KiB, 2 to 4 reads in flight (rsdirstat found 4 readers as fast as 8) |
| Live volume | Records change during the read; a torn record fails its fixup check | Skip and count records with bad signature or fixups. Extents are taken at the start, so files created during the scan are missed. The USN catch-up after the scan (section 6) covers them |
| Fragmented `$MFT` | Many extents | Retrieval pointers handle any number of extents. Read extents in parallel |
| Attribute lists | Large or fragmented files and many hard links spill attributes into extension records | Merge every in-use record whose base reference is non-zero into its base. Take sizes only from the `$DATA` piece with `LowestVcn == 0`. Collect `$FILE_NAME` attributes from extension records too. `$ATTRIBUTE_LIST` itself need not be parsed |
| Deleted and reused records | Record not in use, or a name's parent reference has a stale sequence number | Skip records without the in-use flag (0x0001). Check the parent sequence number against the parent record's sequence. On mismatch, attach to an "orphans" bucket, as rsdirstat does |
| Hard links | One record, `LinkCount` > 1, several `$FILE_NAME` attributes with different parents | Index every name (search must find each). Charge bytes once to a **deterministic** owner, for example the name with the lowest (parent id, name). Flag the others as `secondary_link`. Document that per-folder sizes can differ from a walker's (WinSxS against System32) |
| 8.3 names | Extra `$FILE_NAME` with namespace 2 (DOS) | Skip namespace 2. Namespace 3 (Win32 and DOS) is the long name. Namespace 0 (POSIX) is kept |
| Unpaired UTF-16 surrogates in names | Lossy UTF-8 conversion breaks round-trip paths | Store names as WTF-8 (what Rust's `OsString` uses internally on Windows) and rebuild `OsString` with `OsStringExt::from_wide` when acting on a file |
| Sparse and compressed files | `AllocatedLength` (offset 0x28) overstates disk use | Use the total-allocated field (offset 0x40) when the attribute flag says compressed or sparse. disk-cleaner over-reported 8.59 GiB on one volume before this fix (secondary) |
| WOF / CompactOS files | Unnamed stream is sparse; data lives in the named stream `WofCompressedData`; reparse tag `IO_REPARSE_TAG_WOF` | Physical size comes from `WofCompressedData` (WinDirStat, verified). Logical size comes from the unnamed stream |
| ADS | Named `$DATA` streams (Zone.Identifier, etc.) | Logical size: unnamed stream only, matching Explorer and the current walker. Allocated size: add named streams' allocation. Keep `ads_bytes` in a column for transparency |
| Resident data | Tiny files stored inside the record | Logical size is the value length. Allocated: WinDirStat uses `(len+7)&~7`. Explorer's "size on disk" shows 0 for these (unverified). Pick one and state it in the docs |
| Cloud placeholders (OneDrive and others) | `RECALL_ON_DATA_ACCESS` / `OFFLINE` in `$STANDARD_INFORMATION`; `IO_REPARSE_TAG_CLOUD_*`; full logical size, near-zero allocation | Keep ReClaim's rule: zero bytes counted (`fsmeta::is_cloud_only`). Still index them for search with a `cloud` flag. rsdirstat counts these as 0 too |
| Reparse points (junctions, symlinks, mount points) | Appear as entries with `$REPARSE_POINT` | The flat MFT pass never follows anything, so targets are not double counted (same as today). Show the link entry with size 0 and a `link` flag. Mounted volumes are scanned as their own volume |
| NTFS metafiles | Records 0 to 23 and `$Extend` (`$UsnJrnl:$J` is sparse; `$BadClus:$Bad` has a logical size equal to the volume) | Exclude from the user tree, or group under one "NTFS metadata" node using allocated sizes only. Never use their logical sizes |
| System Volume Information | The walker cannot open it without admin. The MFT sees shadow-copy diff files (can be tens of GB) | Show as its own node with a note. Exclude it when comparing MFT and walker totals |
| BitLocker | Raw reads go through the volume stack | The BitLocker filter decrypts every read "in real time, sector by sector" (Microsoft, 2008 R2 era docs, secondary). Unlocked volumes should read normally. Locked volumes cannot be opened, so use the walker or skip. Verify on a BitLocker test VM (unverified) |
| ReFS (Dev Drive), FAT32, exFAT, network, MTP, optical | No MFT | Fast walker. Detect with `GetVolumeInformationW` (file system name) and `GetDriveTypeW` (fixed, removable or remote) |
| Removable or attacker-supplied NTFS (USB stick) | An elevated parser reads attacker-controlled bytes | MFT mode only for fixed internal volumes by default. Bounds-check every offset and fuzz the record parser (section 9) |
| Memory | Loading the whole `$MFT` costs about 1 KiB per record: 9 GiB on this C: | Stream: parse each 4 MiB buffer and keep only the compact columns (section 6.3) |
| SATA/HDD warm runs | MFT read can be slower than a cached walker (secondary) | Cost check: `$MFT` bytes from `FSCTL_GET_NTFS_VOLUME_DATA` (works without admin) against bus type (`IOCTL_STORAGE_QUERY_PROPERTY`) and the last walk time. disk-cleaner tried probing read throughput and reverted it because the probe evicted the walker's cache |

---

## 5. Rust crates

| Crate | Version (date) | License | Live volume | Sizes | Status and notes | Use in ReClaim |
|---|---|---|---|---|---|---|
| `ntfs-reader` (kikijiki) | 0.6.0 (2026-10-01) | MIT OR Apache-2.0 | Yes, `\\.\C:`, admin; also flat images | Logical `size` only. No allocated size in the docs | Very active (8 releases since March 2026). MFT and USN journal. `Mft::new` is about 1 KiB per file in RAM. `MftScan` uses a few MiB but **reads `$MFT` twice**. `new_compact` saves 30 to 70%. Hard links through `names()`, ADS through `data_streams()`. Depends on `windows` ^0.62 (same as ReClaim) | **Test oracle** (dev-dependency): cross-check names, parents and logical sizes. Not the production scanner |
| `usn-journal-rs` (wangfu91) | 0.5.0 (2026-10-13) | MIT | Yes, admin | No | `FSCTL_ENUM_USN_DATA` iterator, journal iterator with builder options, path resolver with LRU cache. NTFS and ReFS journals | Optional. The journal part is about 200 lines with the `windows` crate. Its README requires admin and does not mention the unprivileged FSCTL (unverified) |
| `ntfs-mft` (wangfu91) | 0.1.0 (2026-10-11) | MIT | Yes, admin | Sizes, hard links, ADS; attribute lists one level | New: 0 stars, 12 recent downloads | Watch only |
| `ntfs` (Colin Finck) | 0.4.0 (2023-06-13) | MIT OR Apache-2.0 | `ntfs-shell` opens `\\.\C:` as admin. Needs a `Read + Seek` that honours sector alignment (unverified) | Through attribute APIs | General per-file library: `no_std`, attribute lists, sparse; no compression. No release since 2023 | Not suited to bulk scans |
| `mft` (omerbenamram) | 0.7.0 (2025-12-23) | MIT/Apache-2.0 | No, parses `$MFT` files or dumps | Yes | Forensic parser, 100% safe Rust, CLI `mft_dump` | Optional fuzz or test oracle over a dumped `$MFT` |
| `windows` | 0.62.2 (already used) | MIT OR Apache-2.0 | n/a | n/a | Has the `FSCTL_*` constants, `NTFS_VOLUME_DATA_BUFFER`, `RETRIEVAL_POINTERS_BUFFER`, `USN_RECORD_V2/V3`, `READ_USN_JOURNAL_DATA_V1`, `MFT_ENUM_DATA_V1`, `QUERY_FILE_LAYOUT_INPUT`, `FILE_LAYOUT_ENTRY`, `FILE_LAYOUT_NAME_ENTRY`, `STREAM_LAYOUT_ENTRY`, `FILE_LAYOUT_INFO_ENTRY`, `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` | **Primary.** Add features `Win32_System_Ioctl` and `Win32_System_IO`. Add `Wdk_Storage_FileSystem` for `NtQueryDirectoryFile`/`NtCreateFile` (feature name unverified) or use the documented `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)` |
| `memchr` | 2.8.3 | Unlicense OR MIT | n/a | n/a | SIMD `memmem` | Substring search over the name pool |
| `regex` | 1.13.1 | MIT OR Apache-2.0 | n/a | n/a | Linear-time, so no ReDoS from user patterns. `regex::bytes` can scan the whole pool | `regex:` queries and case-insensitive literals |
| `globset` | 0.4.20 | Unlicense OR MIT | n/a | n/a | Glob to regex | `*.mp4`, `IMG_????.jpg` |
| `aho-corasick` | 1.1.5 | Unlicense OR MIT | n/a | n/a | Many literals at once, ASCII case-insensitive | `a|b|c` name ORs |
| `roaring` | 0.11.5 | MIT OR Apache-2.0 | n/a | n/a | Compressed bitmaps | Optional; cached per-category sets |
| `lz4_flex` | 0.14.0 | MIT | n/a | n/a | Pure Rust | Index cache and pipe frames |
| `zerocopy` / `bytemuck` | 0.8.63 / 1.25.2 | permissive (BSD-2 / Apache / MIT; Zlib / Apache / MIT) | n/a | n/a | Safe casts of `#[repr(C)]` records | Parsing MFT records without `unsafe` pointer casts, loading cache columns |
| `interprocess` | 2.4.5 | 0BSD OR Apache-2.0 | n/a | n/a | Named pipes | Optional. The `windows` crate is enough, and the security attributes must be set by hand anyway |
| `nucleo-matcher` | 0.3.1 | **MPL-2.0** | n/a | n/a | Fuzzy matcher | Avoid unless fuzzy matching is wanted and MPL's file-level terms are accepted |
| `@tanstack/react-virtual` | 3.14.20 (npm, modified 2026-10-15) | MIT (unverified, TanStack's usual license) | n/a | n/a | Headless `useVirtualizer` | Result table |

Production choice: own reader in `crates/core/src/ntfs/` (about 1,000 to 1,500 lines with tests), built on `windows` and `zerocopy`. Reasons: it needs allocated and WOF sizes and a single pass, no existing crate gives both, and the parsing subset is small and well understood.

---

## 6. Recommended architecture

### 6.1 Overview

```
            UI process (medium IL, owns index + search)
  +-----------------------------------------------------------------+
  | Index store (per volume, columnar)  <-- load/save cache file    |
  |    ^             ^                    ^                         |
  |    | full build  | full build         | live deltas             |
  |    |             |                    |                         |
  | Elevated helper  Fast walker          Journal follower          |
  | (raw $MFT, one   (NtQueryDirectoryFile, (FSCTL_READ_UNPRIVILEGED_|
  |  UAC prompt,      no admin, any FS)     USN_JOURNAL every 1-2 s, |
  |  named pipe)                             no admin, NTFS)         |
  +-----------------------------------------------------------------+
        Search engine (columns -> name -> path -> sort/page)
        Explorer views (tree, treemap, largest, types) from same index
```

How a volume is chosen:

- Local fixed NTFS:
  - If the app is elevated, or the user approves a UAC prompt for the helper, use the MFT reader.
  - Otherwise, use the fast walker.
- Everything else (ReFS/Dev Drive, FAT32, exFAT, removable, network) uses the fast walker.
- Subfolder scans use the index subtree when a fresh index covers the folder. Otherwise they use the fast walker on that folder.

### 6.2 Lifecycle: no service, refreshed when the app opens

1. **App start.**
   - Load `%LOCALAPPDATA%\ReClaim\index\<volume-serial>.idx` (lz4 columns) and show it immediately, marked "as of <time>".
   - Query the journal without admin: `FSCTL_QUERY_USN_JOURNAL` on `C:\`.
2. **Catch-up.** Possible when the journal ID is unchanged and the saved USN is at or after `FirstUsn`. Read the unprivileged journal from the saved USN and apply deltas. On this machine 381k records took 79 ms. The 32 MB journal covered 0.8 h, though, so after a longer gap the next step is the normal path.
3. **Full rebuild.** Used when catch-up is impossible or the user presses Refresh.
   - Elevated MFT read through the helper (one UAC prompt) if the user enabled "Use administrator scan".
   - Fast walker otherwise.
   - The old index stays searchable until the new one replaces it.
4. **While the app is open.** A background thread reads the unprivileged journal every 1 to 2 s from the last USN. For each record:
   - `FILE_DELETE`: remove the entry by FRN.
   - Create, `RENAME_NEW_NAME`: the parent FRN is known in the index, so re-list that one folder with the fast walker. Its `FileIdFullDirectoryInformation` gives each child's FRN, so new and renamed names appear.
   - `DATA_EXTEND`, `DATA_TRUNCATION`, `DATA_OVERWRITE`, `BASIC_INFO_CHANGE` on a known FRN: re-stat by the indexed path (`GetFileAttributesExW`, or `GetFileInformationByHandleEx` for the allocated size).
   - FRN unknown and parent unknown: mark the volume "needs rescan" when there are too many.
   - Coalesce by FRN per tick so a busy build folder costs one re-list per tick.
5. **App exit.** Save the cache with journal ID and next USN. Write it atomically (temp file and rename) in the per-user profile.
6. **No standing elevation.** The helper exits when its pipe closes.

Alternatives considered and rejected:

- A Windows service like Everything's: always running, and the user does not want that.
- A scheduled task "run with highest privileges" for silent elevation: a standing elevated entry point any same-user process can trigger. It is also persistent configuration.
- Growing the journal with `fsutil usn createjournal m=…`: changes system configuration.

### 6.3 Index data structures and memory

Columnar layout (structure of arrays). Entry ids are positions. Entries are stored **grouped by parent**: a counting sort on the parent id after the read. The children of a folder are then one contiguous range, and the explorer tree needs no `Vec<u32>` per node.

```rust
pub struct VolumeIndex {
    // per entry (files and folders), n = entry count
    parent: Vec<u32>,     // 4  entry id of the parent; root = 0
    name_off: Vec<u32>,   // 4  offset into `names` (pool < 4 GiB)
    name_len: Vec<u16>,   // 2  WTF-8 bytes (NTFS max 255 UTF-16 units -> <= 765 bytes)
    flags: Vec<u16>,      // 2  dir, hidden, system, readonly, link, cloud, compressed, sparse,
                          //    secondary_link, has_ads, metafile, denied, resident
    ext: Vec<u16>,        // 2  interned lowercase extension id (0 = none); ext -> category table
    size: Vec<u64>,       // 8  logical bytes (unnamed stream)
    alloc_clusters: Vec<u32>, // 4  allocated clusters incl. ADS/WOF (x cluster size); optional
    mtime: Vec<u32>,      // 4  Unix seconds, unsigned (to 2106); 0 = unknown/implausible
    names: Vec<u8>,       // ~20 avg  WTF-8 names, concatenated, no separator
    // per folder only (about 10-15% of entries)
    dir_first_child: Vec<u32>, dir_child_count: Vec<u32>,   // 8
    dir_bytes: Vec<u64>, dir_alloc: Vec<u64>, dir_files: Vec<u32>, // 20
    // FRN mapping for journal deltas
    rec_to_entry: Vec<u32>, // 4 x MFT record slots (9.43M here = 38 MB); u32::MAX = none
    ext_names: Vec<String>, ext_category: Vec<Category>,
    journal_id: u64, next_usn: i64, cluster_size: u32, built_at: i64, source: Source,
}
```

Per-entry estimate (unverified; measure the average name length on C: in Phase 0):

| Part | Bytes per entry |
|---|---|
| Fixed columns (`parent`, `name_off`, `name_len`, `flags`, `ext`, `size`, `alloc_clusters`, `mtime`) | 30 |
| Names (WTF-8, about 20 B average assumed) | 20 |
| Folder aggregates (28 B x about 12% folders) | 3.4 |
| `rec_to_entry` (4 B x 1.4 record slots per entry here) | 5.6 |
| **Total** | **about 59 B, so about 59 MB per million entries** |
| Lean variant (no `alloc_clusters`, `size` as u48, front-coded names) | about 45 MB per million (unverified) |

- For C: here, about 6.5 to 7M entries (5.8M files plus folders) means **300 to 420 MB**.
- Everything's documented 100 MB per million includes its fast-sort data, so this is in the same range.
- Today's explorer `Node` (`String` + `Vec<u32>` + 11 more fields, about 120 B plus heap per node) disappears once the explorer reads the index. Part of the cost is therefore offset.

Build-time peak:

- One 4 MiB buffer per reader.
- Per parsed record before the parent sort, about 48 B plus the name.
- One `u32` permutation.
- Peak is about 1.5 times the final index (unverified).
- Never hold raw records (1 KiB each).

Query-time scratch:

- `Vec<u32>` of matches (4 B per hit).
- Folder-path cache for `path:` queries, built only for folders.

### 6.4 Search engine

Query pipeline: **parse, plan, execute, rank, page.**

1. **Parse** an Everything-compatible subset into an AST:

| Syntax | Meaning | Executed on |
|---|---|---|
| `foo bar` | AND of substrings (case-insensitive) | `names` pool |
| `foo | bar`, `!foo`, `< >` grouping, `"exact phrase"` | OR, NOT, grouping, literal with spaces | AST |
| `*.mp4`, `IMG_????.jpg` | Wildcards match the **whole** name (Everything behaviour) | `globset` to regex, then per name |
| `regex:^IMG_\d+` | Regex on the name | `regex::bytes` |
| `case:` / `wholeword:` / `path:` / `file:` / `folder:` | Modifiers | flags, path builder |
| `ext:jpg;png` | Extension set | `ext` column (bitset over ext ids) |
| `type:video` or macros `audio: video: image: doc: zip: exe:` (Everything names) plus ReClaim's own (`code:`, `cache:`) | Category from `categories.rs` | `ext -> category` table |
| `size:>100mb`, `size:1gb..5gb`, `size:<1kb` | Size range (folders: total size) | `size` / `dir_bytes` |
| `dm:today`, `dm:7days`, `dm:2025`, `dm:2025-07-01..2026-06-30` | Modified date | `mtime` |
| `parent:C:\Users\me\Downloads` / `infolder:` | Direct children / subtree | id ranges via `dir_first_child` / subtree walk |
| `path:node_modules` or a term containing `\` | Match against full path | folder-path cache |
| `attrib:H`, `cloud:`, `link:` | Attribute flags | `flags` |
| `dupe:` | Same lowercase name and same size (candidates) | sort by (size, name hash) |
| `sort:size-desc`, `top:100` | Order and limit | sort/select |

2. **Plan:** cheapest filters first.
   - Columnar predicates (flags, ext bitset, size, mtime) run as branch-light loops over `rayon` chunks.
   - Then name matching on survivors. When nothing narrows the set first, scan the whole pool once with `memchr::memmem::Finder` or `regex::bytes::Regex::find_iter`, and map match offsets to entries by binary search in `name_off`. This is the same "strstr over every name" Everything uses.
   - Then path predicates.
   - Then `dupe:`.
3. **Path predicates.**
   - Build each folder's full path once per query, in parent order. Folders are about 1M here, so this takes tens of milliseconds (unverified).
   - Test the needle against `folder_path + "\" + name` only for survivors.
4. **Case-insensitivity.** ASCII fast path. `(?i)` in `regex` gives Unicode simple case folding. NTFS uses its `$UpCase` table, and small differences are acceptable.
5. **Rank and page.**
   - Default order is size descending (a disk tool). Name, path and date sorts are also available.
   - Use `par_sort_unstable_by_key` on the match set, or `select_nth_unstable` for `top:`.
   - The result set stays in Rust. The UI asks for pages.
6. **Targets** (unverified; benchmark on a synthetic 7M-entry index): plain substring under 30 ms; regex under 150 ms; `ext:`+`size:` under 20 ms; first page returned under 50 ms at p95.

Features built on the same index:

- **Largest files of type X**: `ext:mp4 sort:size-desc top:200`, or a "Largest videos" preset chip.
- **Type breakdown**: one pass over `ext` and `size` into `Vec<(u64 bytes, u64 count)>` indexed by ext id, then rolled up by category. This replaces today's `Mutex<HashMap<String,…>>` per folder.
- **Duplicates.**
  - `dupe:` finds name+size candidates in milliseconds.
  - "Verify contents" passes the candidate paths to the existing `duplicates.rs` (xxh3 prefix, then blake3).
  - Size-only groups (Everything 1.5's "Find Size duplicates", verified) work the same way.
- **Explorer**: tree, treemap and "largest" views read `dir_*` columns and contiguous child ranges. Small files can still be folded per folder at view time.

### 6.5 Helper protocol (elevated MFT read)

- Command line: `--elevated-index --pipe <name> --ui-pid <pid> --volumes C,D`.
  - Pipe name must match `^\\\\\.\\pipe\\reclaim-index-[0-9a-f]{32}$`.
  - Volumes are drive letters only.
  - The helper re-validates each volume itself: `GetDriveTypeW == DRIVE_FIXED`, `GetVolumeInformationW` reports `NTFS`, not removable by default.
  - No paths, globs, rule ids or actions are accepted. The helper never deletes, writes or changes journals.
- Handles: the volume opens with `FILE_READ_DATA | FILE_READ_ATTRIBUTES`, and `$MFT` with `FILE_READ_ATTRIBUTES` for retrieval pointers. Nothing else.
- Output stream:
  - Header: magic, protocol version, volume serial, journal ID and next USN taken **before** the read so the UI can catch up after it, cluster size, record size.
  - Then length-prefixed lz4 frames of compact records: record number, sequence, parent reference, flags, sizes, mtime, attributes, names.
  - Then an end frame with counts and an xxh3 checksum.
- The UI treats the stream as untrusted input: bounds-check every length, cap the name length (765 B), cap the total size, verify the parent ids.
- The helper exits on pipe close, on error, or after a hard timeout of about 5 minutes per volume.

---

## 7. Passing the index from the elevated helper to the UI

| Transport | How | Pros | Cons |
|---|---|---|---|
| **Named pipe, UI is the server (recommended)** | UI creates `\\.\pipe\reclaim-index-<128-bit random>` before launching the helper. Helper connects as client and streams | Streams progress, no data at rest, UI controls the DACL, squatting impossible because the UI creates it first | Needs careful flags (below) |
| Temp file (current clean helper pattern) | Helper writes `data_dir\elevated\index-<stamp>.bin` with `create_new` after checking the parent is a real folder | Simple; same code style as `helper.rs` | Hundreds of MB written and read again; data at rest until deleted; no streaming progress |
| Shared memory section | UI creates a pagefile-backed `CreateFileMappingW` with a DACL and passes its name | Zero copy | Size unknown in advance (needs `SEC_RESERVE` growth or a second pass), more complex, and no real gain over a pipe at about 1 GB/s |
| Anonymous pipe or inherited handle | Not possible: ShellExecuteEx `runas` goes through the AppInfo service, so the elevated child cannot inherit the UI's handles (unverified, widely reported) | — | — |

Named pipe settings:

- **UI side.** `CreateNamedPipeW` with:
  - `PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED` and `PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS`, with `nMaxInstances = 1`.
  - An explicit security descriptor that allows only the current user SID, plus the logon SID per Microsoft's advice for blocking other sessions, and SYSTEM.
  - The default pipe DACL grants read to Everyone and anonymous (verified, Microsoft Learn), so never pass `NULL`.
  - Do not grant `FILE_CREATE_PIPE_INSTANCE` (it shares its bit with `FILE_APPEND_DATA`; the docs warn about `FILE_GENERIC_WRITE` for this reason).
- **Helper side.**
  - `CreateFileW(pipe, GENERIC_WRITE, 0, …, SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)`. Without SQOS the server impersonates the client at `SecurityImpersonation` by default (verified, Microsoft Learn), and an elevated client must not hand that out.
  - Then `GetNamedPipeServerProcessId` must equal `--ui-pid`.
  - That PID's image (`QueryFullProcessImageNameW`) must equal the helper's own exe path.
- **UI side check.** `GetNamedPipeClientProcessId` must equal the PID of the process it launched (`ShellExecuteExW` with `SEE_MASK_NOCLOSEPROCESS`, then `GetProcessId`).

Security implications:

1. **Privilege boundary: file names leave the admin context.**
   - The index lists names, sizes and dates (not contents) of every file, including other users' profiles and ACL-protected folders.
   - With UAC admin approval mode, the elevated token is the same user, so this is acceptable once the docs and FAQ state it clearly.
   - With over-the-shoulder elevation (a standard user types an administrator's password), the helper would leak other users' file names to a standard user. The helper should compare its token user SID with the UI process's user SID (`OpenProcessToken` on `--ui-pid`). If they differ, refuse MFT mode and let the UI use the walker. Everything has this exposure (users report seeing another account's files, secondary).
2. **Confused deputy.**
   - Any same-user process can launch `reclaim-cleaner.exe --elevated-index …`. The UAC prompt still appears.
   - Because the pipe server must be a ReClaim image with the given PID, output only goes to a ReClaim UI.
   - Same-user malware can still inject into the UI. That is not a boundary ReClaim can defend, and the read-only helper limits the damage to name disclosure.
3. **Elevated parser attack surface.** A crafted NTFS volume (USB stick) could exploit a parser bug in an elevated process.
   - Default MFT mode to fixed internal disks.
   - Parse with `zerocopy`, use no `unsafe` pointer arithmetic, and bounds-check every offset.
   - Fuzz the record parser.
   - Run the helper with the smallest handle set.
4. **Index at rest.**
   - The cache lists every file name, so keep it in `%LOCALAPPDATA%`. The per-user ACL is inherited, and it must not roam.
   - Add "Delete search index" in settings and exclude the cache from logs and crash reports.
   - Do not write any of it into `history.jsonl`.
5. **Helper accepts no paths.** This is unchanged from the current `--elevated-clean` design: drive letters only, re-validated inside the helper, and the helper only reads.

---

## 8. UI patterns for search and large result sets

- **Search bar plus chips.**
  - One input with the Everything-style syntax and debounced queries (60 to 100 ms).
  - Each new query carries a generation number, and stale results are dropped.
  - Filter chips (Type, Size, Modified, Location, Attributes) **write the same syntax** into the input, so chips and text stay in sync and the text query is shareable.
- **Results table.**
  - Virtualized with `@tanstack/react-virtual` (`useVirtualizer`; count = total, fixed row height 28 to 32 px).
  - Columns: Name, Folder, Size, Modified, Type.
  - Clicking a column header sorts in Rust.
  - Very large counts: Chromium limits element height to roughly 33.5M px (unverified for WebView2), and 5.8M rows x 28 px would exceed it. For more than about 1M results, scale the scroll position or show "first 1,000,000, refine the query".
- **Paging, not dumping.**
  - `search(query) -> { queryId, total, totalBytes, firstPage }` and `search_page(queryId, offset, count)`.
  - The result `Vec<u32>` lives in `AppState`.
  - JSON pages of 100 to 300 rows are small.
  - If profiling shows JSON cost, return `tauri::ipc::Response` or send `InvokeResponseBody::Raw`. Both arrive as an `ArrayBuffer` (verified, Tauri docs). `Channel<Vec<u8>>` is sent as a JSON number array (verified), so avoid it.
- **Channels for streams.** Use `tauri::ipc::Channel` for scan progress (helper frames received, records parsed, walker folders) and for "index updated: +N changes" while the journal follower runs. Tauri's docs say events are "not designed for low latency or high throughput" and channels "are designed to be fast and deliver ordered data" (verified).
- **Type breakdown.**
  - A horizontal bar list by category, expandable to extensions, with bytes and counts, top N plus "Other".
  - Optionally a treemap of category to extension.
  - Clicking a bar sets `ext:` or `type:` in the search.
  - Colors and tokens come from `house-design`. Charts follow the `dataviz` skill.
- **Index status line.** For example "C: 6.7M items, MFT scan 14:02, live" or "D: walker scan 13:40, stale: Refresh". Show a shield icon when the refresh needs UAC.
- **Actions.**
  - Enter opens, Shift+Enter reveals in Explorer, Delete sends to the Recycle Bin through the existing clean path with its confirmation and guard.
  - Copy path. "Add to clean list".
  - Keyboard first with visible focus, matching `docs/research.md`.

---

## 9. Test strategy

1. **Unit tests for the record parser.** Synthetic 1 KiB records built in tests:
   - fixups (good and torn);
   - resident and non-resident `$DATA`; sparse, compressed and WOF `WofCompressedData`;
   - several `$FILE_NAME` (POSIX, Win32, DOS, Win32 and DOS);
   - extension records with `LowestVcn != 0` pieces;
   - reparse tags; not-in-use records; bad attribute lengths and out-of-bounds offsets (must return errors, never panic).
2. **Fuzzing.** `cargo fuzz` target on `parse_record(&[u8; 1024|4096])` and on the helper-frame decoder. Run in CI for a fixed time budget.
3. **Golden VHDX tests (Windows CI runner, admin).**
   - Create a 1 to 4 GB VHDX, format NTFS, mount it to a letter.
   - Populate a known tree:
     - hard links (`mklink /H`, including 300 links to one file to force an attribute list);
     - ADS (`echo x > f.txt:s`);
     - sparse (`fsutil sparse setflag/setrange`), NTFS compression (`compact /c`), WOF (`compact /c /exe:lzx`);
     - junctions and symlinks; 8.3 on and off (`fsutil 8dot3name set`);
     - names with unpaired surrogates (through the API); paths over 260 characters;
     - a heavily fragmented file; 200k small files.
   - Compare the MFT reader, the fast walker and `ntfs-reader` (dev-dependency) against the expected manifest: names, parents, logical sizes, allocated sizes, link owners.
   - Dismount at the end.
4. **Live comparison on the developer's volumes (manual, script in `scripts/`).**
   - Run back to back on a quiet system: the elevated walker (so ACL-blocked folders are readable), then the MFT reader. Do C: (NVMe) and D: (SATA).
   - Compare: total files, total logical bytes, total allocated bytes, per top-level folder bytes, and the share of folders that match to the byte.
   - Expected differences to classify, not hide: System Volume Information, hard-link owner choice, files changed between the two runs, NTFS metafiles, cloud placeholders.
   - Pass criteria: totals within 0.1% excluding System Volume Information; at least 97% of folders byte-identical (rsdirstat reached 97.4%; disk-cleaner reached byte-for-byte totals after its fixes).
   - Cross-check allocated total plus metafiles against used clusters (`TotalClusters - FreeClusters` from `FSCTL_GET_NTFS_VOLUME_DATA`).
   - Optional manual check against WizTree's CSV export (personal-use license).
5. **Performance harness.** `crates/core/examples/scanbench.rs` prints JSON: method, volume, bus type, cold or warm, seconds, entries, `$MFT` bytes, peak working set (`GetProcessMemoryInfo`).
   - Cold means after a reboot.
   - Record the current debug walker, the release walker, the fast walker and the MFT reader on C: and D:. This settles the SATA cost check with data.
6. **Incremental correctness.**
   - A script creates, renames, moves, extends, truncates and deletes files and folders while the journal follower runs. The index must then equal a fresh scan.
   - Journal wrap: on the test VHDX, `fsutil usn createjournal m=1048576 a=65536 X:` plus churn must trigger "needs rescan", not a silently wrong index.
7. **Search tests.**
   - Parser golden tests for each syntax row in section 6.4, and property tests (random AST to string to AST).
   - Speed benchmarks on a synthetic 7M-entry index generated from a real name distribution.
8. **Security tests.** The helper rejects:
   - paths, UNC names, non-NTFS, removable or unknown letters;
   - a pipe whose server PID or image does not match;
   - the over-the-shoulder case where the user SIDs differ.

   The UI rejects malformed, oversized or truncated frames, and the pipe's DACL contains no Everyone ACE.

---

## 10. Phased plan

| Phase | Work | Effort (1 developer, unverified) | Exit criteria |
|---|---|---|---|
| 0. Measure and quick fixes | Release-build timing and the `scanbench` harness. In the current walker: skip per-file `String`/`PathBuf` allocations (only big files need names), compute extensions on `OsStr` bytes without `to_lowercase` allocation, use a per-thread ext map merged with `rayon` reduce, and set `FIND_FIRST_EX_LARGE_FETCH` (needs a custom `FindFirstFileExW`). Measure the average name length | 2 to 3 days | Release numbers for C: and D:, cold and warm, recorded in `docs/` |
| 1. Fast non-admin walker | `NtQueryDirectoryFile` (or `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)`) with 1 to 4 MiB thread-local buffers (64 KiB on remote), handles opened relative to the parent. Collect allocated size, file ID and reparse tag. Count hard links once by sorting file IDs after the scan (8 B per file, temporary). Same output as before | 4 to 6 days | Equal totals to the old walker except hard links, and documented speedup |
| 2. Elevated MFT reader | `ntfs` module (volume data, retrieval pointers, overlapped aligned reads, record parser, extension merge, sizes, WOF, hard-link owner), `--elevated-index` helper with the named-pipe protocol, cost check, fallback, fuzz target, golden VHDX tests. 1 to 2 day spike on `FSCTL_QUERY_FILE_LAYOUT` at the start | 8 to 12 days | Section 9.4 criteria met on C: and D:, about 5 s on C: (target) |
| 3. Unified index and search | Columnar `VolumeIndex` built by both scanners, explorer views moved onto it, query parser and planner, Search page with virtualized table, chips and type breakdown, paging commands | 8 to 12 days | Section 6.4 latency targets met; explorer unchanged for users |
| 4. Persistence and live updates | lz4 cache save/load, unprivileged journal follower, catch-up on open, wrap and ID detection, "needs rescan" UX, settings toggles (instant search on/off, admin scan on/off, delete index) | 6 to 9 days | Section 9.6 tests pass; a cached index shows within 1 s of start (target, unverified) |
| 5. Polish and docs | `dupe:` to content verification, "Largest of type" presets, docs (architecture, FAQ: privacy of the index, admin prompt, sizes), CHANGELOG, ROADMAP | 3 to 5 days | Docs and FAQ updated in the same change, as the global rules require |

Total: about **6 to 9 weeks**. Phases 0 and 1 already help every user without admin. Phases 3 and 4 do not depend on Phase 2: the walker can feed the index first.

---

## 11. Risks

| Risk | Likelihood / impact | Mitigation |
|---|---|---|
| Parser bugs on unusual volumes give wrong sizes | Medium / high (wrong numbers in a cleaner) | Walker fallback, golden VHDX matrix, live cross-checks, `ntfs-reader` oracle, "Verify with folder scan" action |
| Elevated parser exploited by a crafted USB volume | Low / high | Fixed disks only by default, safe parsing, fuzzing, minimal handles |
| MFT slower than the walker on SATA/HDD warm runs | Medium / low | Cost check from measured data (Phase 0 and 2 benchmarks) |
| Index memory 300 to 420 MB on a 6 to 7M-entry C: | High / medium | Opt-in instant search, lean columns, drop the old explorer arena, evaluate front-coding |
| Privacy: the index exposes all file names (other users) | Medium / medium | SID check in the helper, per-user cache, opt-in, clear docs and FAQ, "Delete index" |
| Journal too short for catch-up (0.8 h measured here) | High / low | Treat catch-up as a bonus; keep the full rebuild fast; follow the journal live while the app is open |
| `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` undocumented; behavior may change | Low-medium / low | Feature-detect at runtime; on error fall back to "stale, refresh" |
| `NtQueryDirectoryFile` is a native API | Low / low | Documented in the WDK; or use the fully documented `FileIdExtdDirectoryInfo` path (its Win32 page lists only Server 2012 as minimum; works on Windows 10/11 in practice, unverified) |
| Hard-link owner choice makes per-folder sizes differ from other tools | High / low | Deterministic owner, `secondary_link` flag, FAQ entry ("deleting one link frees nothing") |
| BitLocker or other volume filters behave unexpectedly | Low / medium | Test VM with BitLocker; fall back on any read error |
| UAC fatigue | Medium / low | Admin scan opt-in; unprivileged live updates need no prompt |

---

## 12. Open questions for a follow-up

- Speed and size semantics of `FSCTL_QUERY_FILE_LAYOUT` against a raw read (Phase 2 spike).
- Whether `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` filters records for files the caller cannot access, and its minimum Windows build.
- Exact Chromium/WebView2 maximum scroll height, and TanStack Virtual behavior beyond it.
- Average WTF-8 name length and folder/file ratio on the developer's C: (feeds the memory estimate).
- How fast the journal wraps on a normal day (0.8 h was measured during heavy build activity).

---

## Sources

- Microsoft Learn: [FSCTL_ENUM_USN_DATA](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data), [FSCTL_QUERY_FILE_LAYOUT](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/fsctl-query-file-layout), [QUERY_FILE_LAYOUT_INPUT](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_query_file_layout_input), [QUERY_FILE_LAYOUT_OUTPUT](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_query_file_layout_output), [FILE_ID_EXTD_DIR_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_extd_dir_info), [Change journals](https://learn.microsoft.com/en-us/windows/win32/fileio/change-journals), [Using the change journal identifier](https://learn.microsoft.com/en-us/windows/win32/fileio/using-the-change-journal-identifier), [MFT_ENUM_DATA_V1](https://learn.microsoft.com/en-us/windows/win32/api/WinIoCtl/ns-winioctl-mft_enum_data_v1), [Named pipe security and access rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights), [Impersonating a named pipe client](https://learn.microsoft.com/en-us/windows/win32/ipc/impersonating-a-named-pipe-client), [BitLocker filter driver (2008 R2)](https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2008-R2-and-2008/dd363965(v=ws.10))
- windows-rs: [FSCTL_READ_UNPRIVILEGED_USN_JOURNAL](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/System/Ioctl/constant.FSCTL_READ_UNPRIVILEGED_USN_JOURNAL.html), [FILE_LAYOUT_ENTRY](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/System/Ioctl/struct.FILE_LAYOUT_ENTRY.html), [STREAM_LAYOUT_ENTRY](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/System/Ioctl/struct.STREAM_LAYOUT_ENTRY.html)
- Rust std Windows `readdir`: [library/std/src/sys/fs/windows.rs](https://github.com/rust-lang/rust/blob/master/library/std/src/sys/fs/windows.rs)
- WinDirStat source (GPL-3.0, read only): [FinderNtfs.cpp](https://github.com/windirstat/windirstat/blob/master/windirstat/FinderNtfs.cpp), [FinderBasic.cpp](https://github.com/windirstat/windirstat/blob/master/windirstat/FinderBasic.cpp); [WinDirStat 2.5 coverage](https://alternativeto.net/news/2026/1/windirstat-2-5-adds-dark-mode-ntfs-mft-scanning-improved-search-and-new-file-actions/)
- WizTree: [diskanalyzer.com](https://diskanalyzer.com/); [WizTree update thread](https://malwaretips.com/threads/wiztree-update-thread.132667/)
- Everything: [FAQ](https://www.voidtools.com/faq/), [Searching](https://www.voidtools.com/support/everything/searching/), [Indexes](https://www.voidtools.com/support/everything/indexes), [developer on the database design](https://voidtools.com/forum/viewtopic.php?p=60262), [multi-user visibility report](https://voidtools.com/forum/viewtopic.php?p=32366)
- Benchmarks: [rsdirstat PR #6](https://github.com/rikshot/rsdirstat/pull/6), [disk-cleaner PR #89](https://github.com/domsleee/disk-cleaner/pull/89), [dirstats PR #10](https://github.com/iikachu/dirstats/pull/10), [Tachyon](https://github.com/cyb3rcr4t0712/tachyon), [UltraFastFileSearch](https://github.com/GanizaniSitara/UltraFastFileSearch), [FSCTL_ENUM_USN_DATA 700k files report](http://julene-lharudhar.blogspot.com/2015/02/c-speed-up-ntfs-file-enumeration-using.html), [FIND_FIRST_EX_LARGE_FETCH measurement](https://blog.s-schoener.com/2024-06-09-find-first-large-fetch/), [vs-chromium NtQueryDirectoryFile issue](https://github.com/chromium/vs-chromium/issues/58), [Whole Tomato attribute query benchmark](https://www.wholetomato.com/blog/how-to-query-file-attributes-50x-faster-on-windows/)
- Crates: [ntfs-reader](https://github.com/kikijiki/ntfs-reader) / [docs](https://docs.rs/ntfs-reader/0.6.0/ntfs_reader/), [usn-journal-rs](https://github.com/wangfu91/usn-journal-rs), [ntfs-mft](https://github.com/wangfu91/ntfs-mft), [ntfs](https://github.com/ColinFinck/ntfs), [mft](https://github.com/omerbenamram/mft); versions and licenses from the crates.io API on 2026-10-16
- Tauri: [Calling the frontend (channels)](https://v2.tauri.app/develop/calling-frontend/), [Calling Rust (Response, raw bodies)](https://v2.tauri.app/develop/calling-rust/), [tauri::ipc::Channel 2.12.4](https://docs.rs/tauri/latest/tauri/ipc/struct.Channel.html)
- TanStack Virtual: [introduction](https://tanstack.com/virtual/latest/docs/introduction); npm `@tanstack/react-virtual` 3.14.20

Web search budget ran out before the Chromium scroll-height limit could be checked. That item stays unverified (section 12).
