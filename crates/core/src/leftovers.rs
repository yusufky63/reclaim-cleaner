//! Leftover scan after an uninstall.
//!
//! Vendor uninstallers often leave folders behind in AppData, ProgramData and Program Files.
//! Following Bulk Crap Uninstaller, every folder there that is named after the removed app or
//! its publisher is scored: the install folder, an exact name match and `Publisher\App` are high
//! confidence; a publisher folder on its own and a similar name are low confidence and left for
//! the user to decide. Folders another installed app may still use are not reported, links are
//! never followed, and every result passes the [`Guard`] for [`Origin::Leftover`].

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::apps::InstalledApp;
use crate::fsmeta::is_link;
use crate::job::Job;
use crate::paths::{Guard, Origin, SHARED_APP_FOLDERS, env_path, is_within, key};
use crate::walk::measure_dir;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// The install folder from the app's registry entry.
    Install,
    /// Named exactly like the app.
    Name,
    /// The app's folder inside its publisher's folder.
    PublisherName,
    /// The publisher's folder; it may hold data of other apps by the same publisher.
    Publisher,
    /// A similar name, such as the first words of the app name.
    Similar,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Leftover {
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    pub confidence: Confidence,
    pub reason: Reason,
    /// Below Program Files or ProgramData, where removing usually needs administrator rights.
    pub admin: bool,
}

/// A folder apps keep their files in.
#[derive(Debug, Clone)]
pub struct Root {
    pub path: PathBuf,
    /// Program Files and ProgramData are shared by all users and usually need admin rights.
    pub admin: bool,
}

/// `%APPDATA%`, `%LOCALAPPDATA%`, `%PROGRAMDATA%` and the Program Files folders.
pub fn default_roots() -> Vec<Root> {
    let mut roots: Vec<Root> = Vec::new();
    for (var, admin) in [
        ("APPDATA", false),
        ("LOCALAPPDATA", false),
        ("PROGRAMDATA", true),
        ("PROGRAMFILES", true),
        ("PROGRAMFILES(X86)", true),
        ("ProgramW6432", true),
    ] {
        if let Some(path) = env_path(var)
            && !roots.iter().any(|r| key(&r.path) == key(&path))
        {
            roots.push(Root { path, admin });
        }
    }
    roots
}

/// Architecture tags installers append to display names.
const ARCH_TAGS: &[&str] = &[
    "x64", "x86", "x86_64", "amd64", "arm64", "64-bit", "32-bit", "64bit", "32bit", "win64",
    "win32",
];

/// Trailing words of publisher names that are not part of the brand.
const COMPANY_WORDS: &[&str] = &[
    "ab",
    "ag",
    "as",
    "asa",
    "bv",
    "co",
    "community",
    "company",
    "contributors",
    "corp",
    "corporation",
    "developers",
    "foundation",
    "gmbh",
    "group",
    "inc",
    "incorporated",
    "kg",
    "kk",
    "limited",
    "llc",
    "ltd",
    "nv",
    "oy",
    "plc",
    "project",
    "pty",
    "sa",
    "sas",
    "software",
    "spa",
    "srl",
    "team",
    "technologies",
];

/// Words too generic to identify an app or a publisher on their own.
const GENERIC_WORDS: &[&str] = &[
    "app",
    "apps",
    "application",
    "cache",
    "client",
    "cloud",
    "common",
    "config",
    "data",
    "desktop",
    "driver",
    "drivers",
    "files",
    "free",
    "game",
    "games",
    "helper",
    "install",
    "installer",
    "launcher",
    "local",
    "manager",
    "microsoft",
    "online",
    "open",
    "player",
    "plugin",
    "plugins",
    "program",
    "programs",
    "roaming",
    "runtime",
    "sdk",
    "server",
    "service",
    "services",
    "settings",
    "setup",
    "shared",
    "software",
    "studio",
    "system",
    "temp",
    "tool",
    "tools",
    "update",
    "updater",
    "user",
    "users",
    "web",
    "windows",
];

/// `17`, `23.01`, `2015-2022`, `v1.2.3`, `3.18.0-beta.1`, but not `7-Zip`.
fn is_version(token: &str) -> bool {
    let t = token.strip_prefix('v').unwrap_or(token);
    let (head, tail) = t.split_once(['-', '+']).unwrap_or((t, ""));
    let numeric = |s: &str| s.chars().all(|c| c.is_ascii_digit() || ".-_".contains(c));
    t.starts_with(|c: char| c.is_ascii_digit())
        && numeric(head)
        && (head.contains('.') || numeric(tail))
}

/// Lowercase words of a display or folder name without bracketed parts, version numbers and
/// architecture tags, so `7-Zip 23.01 (x64)` and the folder `7-Zip` both give `7 zip`.
fn words(s: &str) -> Vec<String> {
    let mut plain = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    plain
        .to_lowercase()
        .split_whitespace()
        .filter(|t| !is_version(t) && !ARCH_TAGS.contains(t))
        .flat_map(|t| t.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Specific enough to point at one app: at least three letters and not a generic first word.
fn distinctive(words: &[String]) -> bool {
    words.first().is_some_and(|first| {
        words.concat().chars().count() >= 3
            && !GENERIC_WORDS.contains(&first.as_str())
            && !first.chars().all(|c| c.is_ascii_digit())
    })
}

/// True when `folder` is the first words of `name` but not all of it: `Zoom` for
/// `Zoom Workplace`.
fn leading(folder: &[String], name: &[String]) -> bool {
    !folder.is_empty()
        && folder.len() < name.len()
        && name.starts_with(folder)
        && distinctive(folder)
}

/// The ways a folder can be named after an app.
struct Names {
    /// `Mozilla Firefox` gives `mozilla firefox`.
    name: Vec<String>,
    /// The name without the publisher in front: `firefox`.
    short: Vec<String>,
    /// The publisher without company words: `Mozilla Foundation` gives `mozilla`.
    publisher: Vec<String>,
}

impl Names {
    fn new(app: &InstalledApp) -> Names {
        let name = words(&app.name);
        let mut publisher = words(&app.publisher);
        if publisher.first().is_some_and(|w| w == "the") {
            publisher.remove(0);
        }
        // `JetBrains s.r.o.` splits into single letters; those go with the company words.
        while publisher.len() > 1
            && publisher
                .last()
                .is_some_and(|w| w.chars().count() == 1 || COMPANY_WORDS.contains(&w.as_str()))
        {
            publisher.pop();
        }
        let short = if !publisher.is_empty()
            && name.len() > publisher.len()
            && name.starts_with(&publisher)
        {
            name[publisher.len()..].to_vec()
        } else if name.len() > 1 && publisher.first() == name.first() {
            name[1..].to_vec()
        } else {
            Vec::new()
        };
        Names {
            name,
            short,
            publisher,
        }
    }

    fn is_name(&self, folder: &[String]) -> bool {
        let f = folder.concat();
        !f.is_empty() && f == self.name.concat()
    }

    /// The full name, or the name without the publisher (`Mozilla\Firefox`).
    fn is_name_or_short(&self, folder: &[String]) -> bool {
        self.is_name(folder) || (!self.short.is_empty() && folder.concat() == self.short.concat())
    }

    fn is_similar(&self, folder: &[String]) -> bool {
        (!self.short.is_empty() && folder.concat() == self.short.concat() && distinctive(folder))
            || leading(folder, &self.name)
            || leading(folder, &self.short)
    }

    fn is_publisher(&self, folder: &[String]) -> bool {
        let f = folder.concat();
        if f.is_empty() || self.publisher.is_empty() {
            return false;
        }
        f == self.publisher.concat()
            || (self.publisher.len() > 1
                && folder == &self.publisher[..1]
                && distinctive(&self.publisher[..1]))
    }

    /// Could this folder belong to the app, by name or by publisher?
    fn claims(&self, folder: &[String]) -> bool {
        self.is_name_or_short(folder) || self.is_similar(folder) || self.is_publisher(folder)
    }
}

/// Real folders directly inside `dir` with their names. Links are skipped, never followed.
fn subfolders(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !is_link(&t)))
        .map(|e| (e.path(), e.file_name().to_string_lossy().into_owned()))
        .collect()
}

fn file_name_words(path: &Path) -> Vec<String> {
    path.file_name()
        .map(|n| words(&n.to_string_lossy()))
        .unwrap_or_default()
}

struct Candidate {
    path: PathBuf,
    key: String,
    confidence: Confidence,
    reason: Reason,
    admin: bool,
}

/// Leftovers of `app` in the [`default_roots`]. `others` are the apps that are still installed;
/// folders they may use are not reported.
pub fn find(app: &InstalledApp, others: &[InstalledApp], guard: &Guard) -> Vec<Leftover> {
    find_in(app, others, &default_roots(), guard)
}

/// Leftovers of `app` directly inside `roots`, or one level deeper inside a publisher folder.
pub fn find_in(
    app: &InstalledApp,
    others: &[InstalledApp],
    roots: &[Root],
    guard: &Guard,
) -> Vec<Leftover> {
    let me = Names::new(app);
    if me.name.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<Candidate> = Vec::new();
    let mut add = |path: PathBuf, confidence: Confidence, reason: Reason, admin: bool| {
        let key = key(&path);
        found.push(Candidate {
            path,
            key,
            confidence,
            reason,
            admin,
        });
    };

    // The registered install folder comes first so it wins over the same folder found by name.
    if let Some(loc) = app.location.as_deref() {
        let path = PathBuf::from(loc.trim_end_matches(['\\', '/']));
        let real_dir = path
            .symlink_metadata()
            .is_ok_and(|m| m.is_dir() && !is_link(&m.file_type()));
        if path.is_absolute() && real_dir {
            let w = file_name_words(&path);
            let in_publisher = path
                .parent()
                .is_some_and(|p| me.is_publisher(&file_name_words(p)));
            // A registered folder that carries neither the app's nor the publisher's name may
            // be a shared parent such as `D:\Games`, so it is only offered, not preselected.
            let confidence = if me.claims(&w) || in_publisher {
                Confidence::High
            } else {
                Confidence::Low
            };
            let k = key(&path);
            let admin = roots
                .iter()
                .any(|r| r.admin && is_within(&k, &key(&r.path)));
            add(path, confidence, Reason::Install, admin);
        }
    }

    for root in roots {
        for (dir, name) in subfolders(&root.path) {
            if root.admin && SHARED_APP_FOLDERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let w = words(&name);
            if me.is_name(&w) {
                add(dir, Confidence::High, Reason::Name, root.admin);
            } else if me.is_publisher(&w) {
                let mut inside = false;
                for (child, child_name) in subfolders(&dir) {
                    let cw = words(&child_name);
                    if me.is_name_or_short(&cw) {
                        add(child, Confidence::High, Reason::PublisherName, root.admin);
                        inside = true;
                    } else if me.is_similar(&cw) {
                        add(child, Confidence::Low, Reason::Similar, root.admin);
                        inside = true;
                    }
                }
                if !inside {
                    add(dir, Confidence::Low, Reason::Publisher, root.admin);
                }
            } else if me.is_similar(&w) {
                add(dir, Confidence::Low, Reason::Similar, root.admin);
            }
        }
    }

    let others: Vec<(Names, Option<String>)> = others
        .iter()
        .filter(|o| o.id != app.id)
        .map(|o| {
            (
                Names::new(o),
                o.location.as_deref().map(|l| key(Path::new(l))),
            )
        })
        .collect();
    let in_use = |c: &Candidate| {
        let w = file_name_words(&c.path);
        others.iter().any(|(names, location)| {
            names.claims(&w)
                || location
                    .as_deref()
                    .is_some_and(|l| is_within(l, &c.key) || is_within(&c.key, l))
        })
    };

    // Highest confidence first, outer folders before inner ones: a folder inside another
    // candidate is only kept when it scores higher, so nothing is listed twice.
    found.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then(a.key.len().cmp(&b.key.len()))
    });
    let mut kept: Vec<Candidate> = Vec::new();
    for c in found {
        let covered = kept.iter().any(|k| {
            k.key == c.key
                || ((is_within(&c.key, &k.key) || is_within(&k.key, &c.key))
                    && k.confidence >= c.confidence)
        });
        if covered || in_use(&c) || guard.check(&c.path, Origin::Leftover).is_err() {
            continue;
        }
        kept.push(c);
    }

    let job = Job::new();
    let mut out: Vec<Leftover> = kept
        .into_iter()
        .map(|c| {
            let m = measure_dir(&c.path, None, &job);
            Leftover {
                path: c.path.to_string_lossy().into_owned(),
                bytes: m.bytes,
                files: m.files,
                confidence: c.confidence,
                reason: c.reason,
                admin: c.admin,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then(b.bytes.cmp(&a.bytes))
            .then(a.path.cmp(&b.path))
    });
    out
}

// Rule expansion and the guard only know Windows paths so far.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, publisher: &str, location: Option<&Path>) -> InstalledApp {
        InstalledApp {
            id: id.into(),
            name: name.into(),
            publisher: publisher.into(),
            version: String::new(),
            installed: None,
            bytes: None,
            location: location.map(|p| p.to_string_lossy().into_owned()),
            icon: None,
            uninstall: None,
            quiet_uninstall: None,
            scope: "machine".into(),
            source: "desktop".into(),
            broken: false,
        }
    }

    fn mkdir(p: &Path) {
        fs::create_dir_all(p).unwrap();
        fs::write(p.join("data.bin"), [7u8; 10]).unwrap();
    }

    fn junction(link: &Path, target: &Path) {
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    /// `Roaming` and `Local` stand in for the AppData folders, `PF` for Program Files.
    struct Fixture {
        _tmp: tempfile::TempDir,
        base: PathBuf,
        roots: Vec<Root>,
        guard: Guard,
    }

    fn fixture(exclusions: &[String]) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        let roots = vec![
            Root {
                path: base.join("Roaming"),
                admin: false,
            },
            Root {
                path: base.join("Local"),
                admin: false,
            },
            Root {
                path: base.join("PF"),
                admin: true,
            },
        ];
        for r in &roots {
            fs::create_dir_all(&r.path).unwrap();
        }
        let guard = Guard::new(exclusions, None).with_app_root(&base.join("PF"));
        Fixture {
            _tmp: tmp,
            base,
            roots,
            guard,
        }
    }

    fn summary(found: &[Leftover], base: &Path) -> Vec<(String, Confidence, Reason, bool)> {
        let mut s: Vec<_> = found
            .iter()
            .map(|l| {
                let rel = Path::new(&l.path).strip_prefix(base).unwrap();
                (
                    rel.to_string_lossy().into_owned(),
                    l.confidence,
                    l.reason,
                    l.admin,
                )
            })
            .collect();
        s.sort_by(|a, b| a.0.cmp(&b.0));
        s
    }

    #[test]
    fn normalizes_names() {
        assert_eq!(words("7-Zip 23.01 (x64)").concat(), "7zip");
        assert_eq!(words("7-Zip").concat(), "7zip");
        assert_eq!(words("Notepad++ (64-bit x64)"), ["notepad"]);
        assert_eq!(words("Python 3.12.1 (64-bit)"), ["python"]);
        assert_eq!(words("Unity Hub 3.18.0-beta.1"), ["unity", "hub"]);
        assert_eq!(words("Half-Life 2 v1.0"), ["half", "life"]);
        assert_eq!(
            words("Microsoft Visual C++ 2015-2022 Redistributable (x64) - 14.38.33135"),
            ["microsoft", "visual", "c", "redistributable"]
        );
        let n = Names::new(&app("a", "Mozilla Firefox (x64 en-US)", "Mozilla", None));
        assert_eq!(n.short, ["firefox"]);
        let n = Names::new(&app("a", "PyCharm", "JetBrains s.r.o.", None));
        assert_eq!(n.publisher, ["jetbrains"]);
        let n = Names::new(&app("a", "Python 3.12", "Python Software Foundation", None));
        assert_eq!(n.publisher, ["python"]);
        let n = Names::new(&app("a", "Epic Games Launcher", "Epic Games, Inc.", None));
        assert_eq!(n.publisher, ["epic", "games"]);
        assert_eq!(n.short, ["launcher"]);
    }

    #[test]
    fn matches_and_scores_folders() {
        let fx = fixture(&[]);
        let b = &fx.base;
        mkdir(&b.join(r"PF\Acme Writer"));
        mkdir(&b.join(r"Roaming\Acme Writer"));
        mkdir(&b.join(r"Local\Acme\Acme Writer"));
        mkdir(&b.join(r"Local\Acme\Other Product"));
        mkdir(&b.join(r"Roaming\Acme"));
        mkdir(&b.join(r"Local\Writer"));
        mkdir(&b.join(r"Roaming\Acme Writer Plugins"));
        mkdir(&b.join(r"Roaming\Unrelated"));
        mkdir(&b.join(r"Local\Common"));
        let target = app(
            "hklm64:acme",
            "Acme Writer 2.1 (x64)",
            "Acme Software Inc.",
            Some(&b.join(r"PF\Acme Writer")),
        );
        let found = find_in(&target, &[], &fx.roots, &fx.guard);
        use Confidence::*;
        use Reason::*;
        assert_eq!(
            summary(&found, b),
            [
                (
                    r"Local\Acme\Acme Writer".to_string(),
                    High,
                    PublisherName,
                    false
                ),
                (r"Local\Writer".to_string(), Low, Similar, false),
                (r"PF\Acme Writer".to_string(), High, Install, true),
                (r"Roaming\Acme".to_string(), Low, Publisher, false),
                (r"Roaming\Acme Writer".to_string(), High, Name, false),
            ]
        );
        assert!(found.iter().all(|l| l.bytes == 10 && l.files == 1));
        assert_eq!(found[0].confidence, High, "high confidence first");
    }

    #[test]
    fn never_follows_links() {
        let fx = fixture(&[]);
        let b = &fx.base;
        let outside = b.join("outside");
        mkdir(&outside.join("Acme Writer"));
        junction(&b.join(r"Roaming\Acme Writer"), &outside);
        junction(&b.join(r"Local\Acme"), &outside);
        let target = app(
            "x",
            "Acme Writer",
            "Acme",
            Some(&b.join(r"Roaming\Acme Writer")),
        );
        let found = find_in(&target, &[], &fx.roots, &fx.guard);
        assert!(found.is_empty(), "{found:?}");
        assert!(outside.join(r"Acme Writer\data.bin").exists());
    }

    #[test]
    fn skips_folders_other_apps_still_use() {
        let fx = fixture(&[]);
        let b = &fx.base;
        mkdir(&b.join(r"Roaming\Acme"));
        mkdir(&b.join(r"Roaming\Acme Writer"));
        mkdir(&b.join(r"Local\Zoom"));
        mkdir(&b.join(r"PF\Suite\Paint"));
        let target = app("a", "Acme Writer", "Acme Inc", None);
        let others = [
            target.clone(),
            app("b", "Acme Paint", "Acme Inc", None),
            app("c", "Acme Writer Pro", "Acme Inc", None),
        ];
        assert!(find_in(&target, &others, &fx.roots, &fx.guard).is_empty());

        let zoom = app("z", "Zoom Workplace", "Zoom Video Communications", None);
        let outlook = app("o", "Zoom Outlook Plugin", "Zoom", None);
        assert_eq!(find_in(&zoom, &[], &fx.roots, &fx.guard).len(), 1);
        assert!(find_in(&zoom, &[outlook], &fx.roots, &fx.guard).is_empty());

        // A folder holding another app's install folder is never offered.
        let suite = app("s", "Suite", "", None);
        let paint = app("p", "Paint", "", Some(&b.join(r"PF\Suite\Paint")));
        assert_eq!(find_in(&suite, &[], &fx.roots, &fx.guard).len(), 1);
        assert!(find_in(&suite, &[paint], &fx.roots, &fx.guard).is_empty());
    }

    #[test]
    fn respects_the_guard() {
        let tmp_fx = fixture(&[]);
        let excluded = tmp_fx.base.join(r"Roaming\Acme Writer");
        let fx = Fixture {
            guard: Guard::new(&[excluded.to_string_lossy().into_owned()], None)
                .with_app_root(&tmp_fx.base.join("PF")),
            ..tmp_fx
        };
        let b = &fx.base;
        mkdir(&excluded);
        mkdir(&b.join(r"PF\Common Files\Acme Writer"));
        mkdir(&b.join(r"PF\Acme\Acme Writer"));
        // A registered install folder that is a shared Windows folder is refused too.
        let target = app(
            "a",
            "Acme Writer",
            "Acme",
            Some(&b.join(r"PF\Common Files")),
        );
        let found = find_in(&target, &[], &fx.roots, &fx.guard);
        assert_eq!(
            summary(&found, b),
            [(
                r"PF\Acme\Acme Writer".to_string(),
                Confidence::High,
                Reason::PublisherName,
                true
            )]
        );
        // Only the leftover origin may reach into the Program Files stand-in.
        let p = Path::new(&found[0].path);
        assert!(fx.guard.check(p, Origin::Leftover).is_ok());
        assert!(fx.guard.check(p, Origin::User).is_err());
    }

    #[test]
    fn scores_the_registered_install_folder() {
        let fx = fixture(&[]);
        let b = &fx.base;
        mkdir(&b.join(r"Games"));
        let target = app("g", "Space Quest", "Sierra", Some(&b.join("Games")));
        let found = find_in(&target, &[], &fx.roots, &fx.guard);
        assert_eq!(
            summary(&found, b),
            [("Games".to_string(), Confidence::Low, Reason::Install, false)]
        );

        // Inside the publisher's folder the registered folder is trusted even with another name,
        // and it wins over the publisher folder around it.
        mkdir(&b.join(r"PF\Ubisoft\Ubisoft Game Launcher"));
        let location = format!(r"{}\PF\Ubisoft\Ubisoft Game Launcher\", b.display());
        let mut target = app("u", "Ubisoft Connect", "Ubisoft", None);
        target.location = Some(location);
        let found = find_in(&target, &[], &fx.roots, &fx.guard);
        assert_eq!(
            summary(&found, b),
            [(
                r"PF\Ubisoft\Ubisoft Game Launcher".to_string(),
                Confidence::High,
                Reason::Install,
                true
            )]
        );
    }
}
