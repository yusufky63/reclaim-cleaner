//! Steam: the install folder, every library folder on every drive, installed games and the
//! per-game shader caches.
//!
//! Steam writes its install path to `HKCU\Software\Valve\Steam` and its library list to
//! `steamapps\libraryfolders.vdf`. Each library keeps `appmanifest_<appid>.acf` files for the
//! games installed there and a `steamapps\shadercache\<appid>` folder per game. Nothing here
//! starts Steam or talks to its servers.

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::fsmeta::is_link;
use crate::job::Job;
use crate::paths::key;
use crate::rules::Risk;
use crate::walk::measure_path;

/// A parsed Valve KeyValues (VDF) value: a string or a nested block.
#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Block(Vec<(String, Vdf)>),
}

impl Vdf {
    /// The value under `name` (case-insensitive, like Steam itself).
    pub fn get(&self, name: &str) -> Option<&Vdf> {
        match self {
            Vdf::Block(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Vdf::Str(s) => Some(s),
            Vdf::Block(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Block(items) => items,
            Vdf::Str(_) => &[],
        }
    }
}

enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => break,
                        },
                        other => s.push(other),
                    }
                }
                out.push(Token::Str(s));
            }
            c if c.is_whitespace() => {}
            // Unquoted tokens (rare in Steam's own files): read up to whitespace or a brace.
            c => {
                let mut s = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                out.push(Token::Str(s));
            }
        }
    }
    out
}

fn parse_block(tokens: &[Token], pos: &mut usize, depth: usize) -> Vec<(String, Vdf)> {
    let mut items = Vec::new();
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Close => {
                *pos += 1;
                break;
            }
            Token::Open => {
                // A block without a key is malformed; skip it as a whole.
                *pos += 1;
                if depth < 32 {
                    parse_block(tokens, pos, depth + 1);
                }
            }
            Token::Str(k) => {
                let k = k.clone();
                *pos += 1;
                match tokens.get(*pos) {
                    Some(Token::Str(v)) => {
                        items.push((k, Vdf::Str(v.clone())));
                        *pos += 1;
                    }
                    Some(Token::Open) if depth < 32 => {
                        *pos += 1;
                        let inner = parse_block(tokens, pos, depth + 1);
                        items.push((k, Vdf::Block(inner)));
                    }
                    _ => break,
                }
            }
        }
    }
    items
}

/// Parses a text VDF file. Malformed input gives whatever could be read, never a panic.
pub fn parse_vdf(text: &str) -> Vdf {
    let tokens = tokenize(text);
    let mut pos = 0;
    Vdf::Block(parse_block(&tokens, &mut pos, 0))
}

/// Library folders listed in `libraryfolders.vdf`. Handles the current format
/// (`"0" { "path" "D:\\SteamLibrary" ... }`) and the pre-2021 one (`"1" "D:\\SteamLibrary"`).
pub fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    let doc = parse_vdf(text);
    let Some(root) = doc
        .get("libraryfolders")
        .or_else(|| doc.get("LibraryFolders"))
    else {
        return Vec::new();
    };
    root.entries()
        .iter()
        .filter(|(k, _)| k.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|(_, v)| match v {
            Vdf::Str(p) => Some(PathBuf::from(p)),
            Vdf::Block(_) => v.str("path").map(PathBuf::from),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub app_id: String,
    pub name: String,
    pub install_dir: String,
    pub size_on_disk: u64,
}

/// Reads an `appmanifest_<appid>.acf` file.
pub fn parse_app_manifest(text: &str) -> Option<Manifest> {
    let doc = parse_vdf(text);
    let state = doc.get("AppState")?;
    let app_id = state.str("appid")?.trim().to_string();
    if app_id.is_empty() || !app_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(Manifest {
        name: state.str("name").unwrap_or_default().trim().to_string(),
        install_dir: state.str("installdir").unwrap_or_default().to_string(),
        size_on_disk: state
            .str("SizeOnDisk")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0),
        app_id,
    })
}

/// A real folder (not a link) that holds a `steamapps` folder (also not a link).
fn is_library(dir: &Path) -> bool {
    let real_dir = |p: &Path| {
        p.symlink_metadata()
            .map(|m| m.is_dir() && !is_link(&m.file_type()))
            .unwrap_or(false)
    };
    dir.is_absolute() && real_dir(dir) && real_dir(&dir.join("steamapps"))
}

fn clean_registry_path(raw: &str) -> Option<PathBuf> {
    let s = raw.trim().replace('/', "\\");
    let p = PathBuf::from(s.trim_end_matches('\\'));
    // Only drive-letter paths: a network share or device path is not a Steam install we clean.
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then_some(p)
}

/// The Steam install folder of the current user, if Steam is installed and the folder is real.
pub fn steam_root() -> Option<PathBuf> {
    let from_user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()
        .and_then(|k| k.get_value::<String, _>("SteamPath").ok());
    let from_machine = || {
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\WOW6432Node\Valve\Steam")
            .or_else(|_| RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Valve\Steam"))
            .ok()
            .and_then(|k| k.get_value::<String, _>("InstallPath").ok())
    };
    from_user
        .as_deref()
        .and_then(clean_registry_path)
        .filter(|p| is_library(p))
        .or_else(|| {
            from_machine()
                .as_deref()
                .and_then(clean_registry_path)
                .filter(|p| is_library(p))
        })
}

/// The Steam folder itself plus every library it lists, each once, in Steam's order. Folders
/// that are gone (an unplugged drive) or are links are left out.
pub fn libraries_in(root: &Path) -> Vec<PathBuf> {
    let listed: Vec<PathBuf> = [
        root.join(r"steamapps\libraryfolders.vdf"),
        root.join(r"config\libraryfolders.vdf"),
    ]
    .iter()
    .filter_map(|f| fs::read_to_string(f).ok())
    .flat_map(|text| parse_library_folders(&text))
    .filter_map(|p| clean_registry_path(&p.to_string_lossy()))
    .collect();
    // Steam stores its own path in lowercase in the registry; the library list keeps the
    // folder's real spelling, which reads better.
    let root = listed
        .iter()
        .find(|p| key(p) == key(root))
        .cloned()
        .unwrap_or_else(|| root.to_path_buf());
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for p in std::iter::once(root).chain(listed) {
        let k = key(&p);
        if !seen.contains(&k) && is_library(&p) {
            seen.push(k);
            out.push(p);
        }
    }
    out
}

/// Every Steam library of the current user; empty when Steam is not installed.
pub fn libraries() -> Vec<PathBuf> {
    steam_root().map(|r| libraries_in(&r)).unwrap_or_default()
}

fn manifests(library: &Path) -> Vec<Manifest> {
    let Ok(rd) = fs::read_dir(library.join("steamapps")) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.starts_with("appmanifest_") && n.ends_with(".acf")
        })
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|t| parse_app_manifest(&t))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGame {
    pub app_id: String,
    pub name: String,
    /// From the manifest's `SizeOnDisk`; the game folder itself is not walked.
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamLibrary {
    pub path: String,
    pub games: Vec<SteamGame>,
    pub bytes: u64,
    pub free: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderCache {
    pub app_id: String,
    /// The game's name from its manifest; `None` when the game is not installed in any library.
    pub name: Option<String>,
    pub installed: bool,
    /// Safe when the game is gone, review when it is installed (it rebuilds, with stutter).
    pub risk: Risk,
    pub library: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SteamReport {
    pub root: Option<String>,
    /// Steam is running; its caches can only be deleted once it is closed.
    pub running: bool,
    pub libraries: Vec<SteamLibrary>,
    pub caches: Vec<ShaderCache>,
}

/// Shader cache folders (`steamapps\shadercache\<appid>`) of one library, unmeasured.
fn cache_dirs(library: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(library.join(r"steamapps\shadercache")) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| {
            e.file_type()
                .map(|t| t.is_dir() && !is_link(&t))
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| (id, e.path()))
        })
        .collect()
}

/// Libraries, installed games and measured shader caches for a Steam folder.
pub fn report_for(root: &Path, job: &Job) -> SteamReport {
    job.set_stage("steam");
    let libs = libraries_in(root);
    let per_lib: Vec<(PathBuf, Vec<Manifest>)> =
        libs.iter().map(|l| (l.clone(), manifests(l))).collect();
    let name_of = |id: &str| {
        per_lib
            .iter()
            .flat_map(|(_, m)| m.iter())
            .find(|m| m.app_id == id)
            .map(|m| m.name.clone())
    };
    let dirs: Vec<(PathBuf, String, PathBuf)> = libs
        .iter()
        .flat_map(|l| {
            cache_dirs(l)
                .into_iter()
                .map(move |(id, p)| (l.clone(), id, p))
        })
        .collect();
    job.set_total(dirs.len() as u64);
    let mut caches: Vec<ShaderCache> = dirs
        .par_iter()
        .map(|(lib, id, path)| {
            job.set_current(path);
            let m = measure_path(path, None, &Job::new());
            job.add_files(m.files, m.bytes);
            job.add_done(1);
            let name = name_of(id).filter(|n| !n.is_empty());
            let installed = name_of(id).is_some();
            ShaderCache {
                app_id: id.clone(),
                name,
                installed,
                risk: if installed { Risk::Review } else { Risk::Safe },
                library: lib.to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
                bytes: m.bytes,
                files: m.files,
            }
        })
        .filter(|c| c.bytes > 0)
        .collect();
    caches.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    let libraries = per_lib
        .into_iter()
        .map(|(path, mut games)| {
            games.sort_by_key(|g| std::cmp::Reverse(g.size_on_disk));
            SteamLibrary {
                bytes: games.iter().map(|g| g.size_on_disk).sum(),
                free: crate::system::free_space(&path),
                path: path.to_string_lossy().into_owned(),
                games: games
                    .into_iter()
                    .map(|g| SteamGame {
                        app_id: g.app_id,
                        name: g.name,
                        bytes: g.size_on_disk,
                    })
                    .collect(),
            }
        })
        .collect();
    SteamReport {
        root: Some(
            libs.first()
                .map_or(root, PathBuf::as_path)
                .to_string_lossy()
                .into_owned(),
        ),
        running: crate::system::running_processes().contains("steam.exe"),
        libraries,
        caches,
    }
}

/// The report for the current user's Steam; empty when Steam is not installed.
pub fn report(job: &Job) -> SteamReport {
    match steam_root() {
        Some(root) => report_for(&root, job),
        None => SteamReport::default(),
    }
}

/// Resolves shader cache folders for the given app ids from Steam's own data, so a request can
/// never name an arbitrary path. Unknown ids are ignored.
pub fn cache_paths_for(root: &Path, app_ids: &[String]) -> Vec<PathBuf> {
    libraries_in(root)
        .iter()
        .flat_map(|l| cache_dirs(l))
        .filter(|(id, _)| app_ids.iter().any(|a| a == id))
        .map(|(_, p)| p)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARIES: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps"
		{
			"228980"		"157818239"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"label"		""
	}
}
"#;

    #[test]
    fn parses_library_folders_in_both_formats() {
        assert_eq!(
            parse_library_folders(LIBRARIES),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary")
            ]
        );
        let old = "\"LibraryFolders\"\n{\n\t\"TimeNextStatsReport\"\t\"1\"\n\t\"1\"\t\"E:\\\\Games\\\\Steam\"\n}";
        assert_eq!(
            parse_library_folders(old),
            vec![PathBuf::from(r"E:\Games\Steam")]
        );
        // Broken input never panics.
        assert!(parse_library_folders("\"libraryfolders\" { \"0\" { \"path\"").is_empty());
        assert!(parse_library_folders("}}}{{").is_empty());
    }

    #[test]
    fn parses_app_manifests() {
        let text = "\"AppState\"\n{\n\t\"appid\"\t\t\"1486920\"\n\t\"name\"\t\t\"Some \\\"Game\\\"\"\n\t\"installdir\"\t\t\"Some Game\"\n\t\"SizeOnDisk\"\t\t\"49721401349\"\n}";
        let m = parse_app_manifest(text).unwrap();
        assert_eq!(m.app_id, "1486920");
        assert_eq!(m.name, "Some \"Game\"");
        assert_eq!(m.size_on_disk, 49_721_401_349);
        assert!(parse_app_manifest("\"AppState\" { \"appid\" \"../x\" }").is_none());
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn finds_libraries_games_and_caches_with_names() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Steam");
        let other = tmp.path().join("Library2");
        let gone = tmp.path().join("Unplugged");
        let vdf = format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} \"2\" {{ \"path\" \"{}\" }} }}",
            root.display().to_string().replace('\\', "\\\\"),
            other.display().to_string().replace('\\', "\\\\"),
            gone.display().to_string().replace('\\', "\\\\"),
        );
        write(&root.join(r"steamapps\libraryfolders.vdf"), &vdf);
        write(
            &other.join(r"steamapps\appmanifest_70.acf"),
            "\"AppState\" { \"appid\" \"70\" \"name\" \"Half-Life\" \"SizeOnDisk\" \"1000\" }",
        );
        write(&other.join(r"steamapps\shadercache\70\a.bin"), "1234567890");
        write(&root.join(r"steamapps\shadercache\999\b.bin"), "12345");
        write(&root.join(r"steamapps\shadercache\notanid\c.bin"), "1");

        assert_eq!(libraries_in(&root), vec![root.clone(), other.clone()]);
        let r = report_for(&root, &Job::new());
        assert_eq!(r.libraries.len(), 2);
        assert_eq!(r.libraries[1].games[0].name, "Half-Life");
        assert_eq!(r.libraries[1].bytes, 1000);
        assert_eq!(r.caches.len(), 2);
        let hl = r.caches.iter().find(|c| c.app_id == "70").unwrap();
        assert_eq!(hl.name.as_deref(), Some("Half-Life"));
        assert!(hl.installed);
        assert_eq!(hl.risk, Risk::Review);
        assert_eq!(hl.bytes, 10);
        let orphan = r.caches.iter().find(|c| c.app_id == "999").unwrap();
        assert!(!orphan.installed && orphan.name.is_none());
        assert_eq!(orphan.risk, Risk::Safe);

        let paths = cache_paths_for(&root, &["999".into(), "12345".into()]);
        assert_eq!(paths, vec![root.join(r"steamapps\shadercache\999")]);
    }

    #[test]
    fn registry_paths_must_be_drive_paths() {
        assert_eq!(
            clean_registry_path("c:/program files (x86)/steam"),
            Some(PathBuf::from(r"c:\program files (x86)\steam"))
        );
        assert!(clean_registry_path(r"\\server\share\Steam").is_none());
        assert!(clean_registry_path("relative/steam").is_none());
    }
}
