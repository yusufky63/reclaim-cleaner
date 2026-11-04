//! The user's folders on Linux: `$HOME`, the XDG base directories and the folders named in
//! `user-dirs.dirs`, which is the Linux counterpart of Windows Known Folders. Their names are
//! often translated, so they are read from that file and never assumed in English.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

/// An absolute path without `.` or `..`, with `//` and trailing slashes gone. The XDG spec says
/// relative values must be ignored, and `..` could point a rule or the guard anywhere.
fn clean_absolute(value: &OsStr) -> Option<PathBuf> {
    let path = Path::new(value);
    if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
        return None;
    }
    Some(path.components().collect())
}

/// `$HOME` and the XDG base directories, each with the spec's default when unset, empty or
/// relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xdg {
    pub home: PathBuf,
    pub cache: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    /// `$XDG_RUNTIME_DIR` has no default.
    pub runtime: Option<PathBuf>,
}

impl Xdg {
    /// `None` when `$HOME` is missing, relative or `/`. A root folder as home would make the
    /// whole system look like the user's own files, so nothing may rely on it.
    pub fn from_vars(get: impl Fn(&str) -> Option<OsString>) -> Option<Xdg> {
        let home = clean_absolute(&get("HOME")?).filter(|h| h.parent().is_some())?;
        let dir = |var: &str, default: &[&str]| {
            get(var)
                .and_then(|v| clean_absolute(&v))
                .filter(|p| p.parent().is_some())
                .unwrap_or_else(|| default.iter().fold(home.clone(), |p, s| p.join(s)))
        };
        Some(Xdg {
            cache: dir("XDG_CACHE_HOME", &[".cache"]),
            config: dir("XDG_CONFIG_HOME", &[".config"]),
            data: dir("XDG_DATA_HOME", &[".local", "share"]),
            state: dir("XDG_STATE_HOME", &[".local", "state"]),
            runtime: get("XDG_RUNTIME_DIR")
                .and_then(|v| clean_absolute(&v))
                .filter(|p| p.parent().is_some()),
            home,
        })
    }

    pub fn current() -> Option<Xdg> {
        Xdg::from_vars(|name| std::env::var_os(name))
    }

    /// Where `user-dirs.dirs` may be: `$XDG_CONFIG_HOME` first, the default folder too.
    fn user_dirs_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.config.join("user-dirs.dirs")];
        let default = self.home.join(".config").join("user-dirs.dirs");
        if !files.contains(&default) {
            files.push(default);
        }
        files
    }

    /// The folders from `user-dirs.dirs`, as `(DESKTOP, path)` pairs. When both files name a
    /// folder, the one in `$XDG_CONFIG_HOME` comes first.
    pub fn user_dirs(&self) -> Vec<(String, PathBuf)> {
        self.user_dirs_files()
            .iter()
            .filter_map(|f| std::fs::read(f).ok())
            .flat_map(|text| parse_user_dirs(&text, &self.home))
            .collect()
    }

    pub fn user_dir(&self, name: &str) -> Option<PathBuf> {
        self.user_dirs()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, p)| p)
    }
}

/// Removes the shell escapes `xdg-user-dirs-update` writes inside the double quotes.
fn unescape(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.iter().copied();
    while let Some(b) = bytes.next() {
        match b {
            b'\\' => out.extend(bytes.next()),
            other => out.push(other),
        }
    }
    out
}

/// Reads `XDG_DOWNLOADS_DIR="$HOME/Downloads"` lines. A value is `$HOME/...` or an absolute path.
/// An entry that is just `$HOME` means the folder is disabled; it is left out, because treating
/// it as a user folder would say nothing and treating it as a target could point at the home
/// folder itself.
pub fn parse_user_dirs(text: &[u8], home: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for line in text.split(|&b| b == b'\n') {
        let line = line.trim_ascii();
        if line.starts_with(b"#") {
            continue;
        }
        let Some(eq) = line.iter().position(|&b| b == b'=') else {
            continue;
        };
        let Some(name) = std::str::from_utf8(&line[..eq])
            .ok()
            .and_then(|n| n.strip_prefix("XDG_")?.strip_suffix("_DIR"))
        else {
            continue;
        };
        let raw = &line[eq + 1..];
        let quoted = raw.len() >= 2 && raw[0] == b'"' && raw[raw.len() - 1] == b'"';
        let value = unescape(if quoted { &raw[1..raw.len() - 1] } else { raw });
        let path = if value == b"$HOME" || value == b"$HOME/" {
            None
        } else if let Some(rest) = value.strip_prefix(b"$HOME/") {
            let mut full = home.as_os_str().to_os_string().into_vec();
            full.push(b'/');
            full.extend_from_slice(rest);
            clean_absolute(OsStr::from_bytes(&full))
        } else {
            clean_absolute(OsStr::from_bytes(&value))
        };
        // `/` or the home folder under another spelling is no user folder either.
        if let Some(path) = path.filter(|p| p != home && p.parent().is_some()) {
            out.push((name.to_string(), path));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn xdg(vars: &[(&str, &str)]) -> Option<Xdg> {
        let map: HashMap<&str, &str> = vars.iter().copied().collect();
        Xdg::from_vars(|name| map.get(name).map(OsString::from))
    }

    #[test]
    fn uses_spec_defaults() {
        let x = xdg(&[("HOME", "/home/u")]).unwrap();
        assert_eq!(x.home, Path::new("/home/u"));
        assert_eq!(x.cache, Path::new("/home/u/.cache"));
        assert_eq!(x.config, Path::new("/home/u/.config"));
        assert_eq!(x.data, Path::new("/home/u/.local/share"));
        assert_eq!(x.state, Path::new("/home/u/.local/state"));
        assert_eq!(x.runtime, None);
    }

    #[test]
    fn honours_absolute_overrides_and_ignores_relative_ones() {
        let x = xdg(&[
            ("HOME", "/home/u/"),
            ("XDG_CACHE_HOME", "/ssd/cache/"),
            ("XDG_CONFIG_HOME", "relative/config"),
            ("XDG_DATA_HOME", ""),
            ("XDG_STATE_HOME", "/home/u/../other"),
            ("XDG_RUNTIME_DIR", "run/user"),
        ])
        .unwrap();
        assert_eq!(x.home, Path::new("/home/u"));
        assert_eq!(x.cache, Path::new("/ssd/cache"));
        assert_eq!(x.config, Path::new("/home/u/.config"));
        assert_eq!(x.data, Path::new("/home/u/.local/share"));
        assert_eq!(x.state, Path::new("/home/u/.local/state"));
        assert_eq!(x.runtime, None);
        let root = xdg(&[("HOME", "/home/u"), ("XDG_CACHE_HOME", "/")]).unwrap();
        assert_eq!(
            root.cache,
            Path::new("/home/u/.cache"),
            "/ is no cache folder"
        );
    }

    #[test]
    fn refuses_a_home_it_cannot_trust() {
        assert_eq!(xdg(&[]), None);
        assert_eq!(xdg(&[("HOME", "")]), None);
        assert_eq!(xdg(&[("HOME", "relative/home")]), None);
        assert_eq!(xdg(&[("HOME", "/")]), None);
        assert_eq!(xdg(&[("HOME", "//")]), None);
        assert_eq!(xdg(&[("HOME", "/home/../etc")]), None);
    }

    #[test]
    fn reads_translated_and_absolute_user_dirs() {
        let text = "# Written by xdg-user-dirs-update\n\
            XDG_DESKTOP_DIR=\"$HOME/Masaüstü\"\n\
            XDG_DOWNLOAD_DIR=\"$HOME/İndirilenler\"\n\
            XDG_TEMPLATES_DIR=\"$HOME/\"\n\
            XDG_PUBLICSHARE_DIR=\"$HOME\"\n\
            XDG_DOCUMENTS_DIR=\"/mnt/data/Belgeler\"\n\
            XDG_MUSIC_DIR=\"$HOME/My\\ Music\"\n\
            XDG_PICTURES_DIR=\"relative/Pictures\"\n\
            XDG_VIDEOS_DIR=\"$HOME/../Videos\"\n\
            OTHER=\"$HOME/x\"\n\
            not a line\n";
        let dirs = parse_user_dirs(text.as_bytes(), Path::new("/home/u"));
        assert_eq!(
            dirs,
            vec![
                ("DESKTOP".to_string(), PathBuf::from("/home/u/Masaüstü")),
                (
                    "DOWNLOAD".to_string(),
                    PathBuf::from("/home/u/İndirilenler")
                ),
                ("DOCUMENTS".to_string(), PathBuf::from("/mnt/data/Belgeler")),
                ("MUSIC".to_string(), PathBuf::from("/home/u/My Music")),
            ]
        );
    }

    #[test]
    fn keeps_user_dirs_that_are_not_utf8() {
        let dirs = parse_user_dirs(b"XDG_MUSIC_DIR=\"$HOME/M\xfcsik\"", Path::new("/home/u"));
        assert_eq!(dirs[0].1.as_os_str().as_bytes(), b"/home/u/M\xfcsik");
    }

    #[test]
    fn prefers_the_file_in_xdg_config_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let other = tmp.path().join("cfg");
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            home.join(".config/user-dirs.dirs"),
            "XDG_MUSIC_DIR=\"$HOME/A\"\nXDG_VIDEOS_DIR=\"$HOME/V\"\n",
        )
        .unwrap();
        std::fs::write(other.join("user-dirs.dirs"), "XDG_MUSIC_DIR=\"$HOME/B\"\n").unwrap();
        let x = Xdg::from_vars(|n| match n {
            "HOME" => Some(home.clone().into_os_string()),
            "XDG_CONFIG_HOME" => Some(other.clone().into_os_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(x.user_dir("MUSIC"), Some(home.join("B")));
        assert_eq!(x.user_dir("VIDEOS"), Some(home.join("V")));
        assert_eq!(x.user_dir("DESKTOP"), None);
    }
}
