//! Space categories for the explorer treemap. A folder inherits its parent's category unless
//! its own name says otherwise, so `node_modules` inside Documents is still code.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Projects,
    Media,
    Caches,
    Apps,
    Ai,
    System,
    Other,
}

const SYSTEM_ROOT_DIRS: &[&str] = &[
    "windows",
    "$recycle.bin",
    "system volume information",
    "recovery",
    "programdata",
    "$windows.~bt",
    "$windows.~ws",
    "$winreagent",
    "$sysreset",
    "perflogs",
    "boot",
    "efi",
    "windows.old",
    "config.msi",
    "msocache",
];
const SYSTEM_ROOT_FILES: &[&str] = &[
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
    "dumpstack.log.tmp",
    "dumpstack.log",
];
const APP_DIRS: &[&str] = &[
    "program files",
    "program files (x86)",
    "steam",
    "steamlibrary",
    "epic games",
    "xboxgames",
    "riot games",
    "gog games",
    "ea games",
    "ubisoft",
    "battle.net",
    "programs",
    "packages",
    "windowsapps",
    ".vscode",
    ".cursor",
    ".vscode-insiders",
    "jetbrains",
    "android",
    ".android",
    "docker",
];
const AI_DIRS: &[&str] = &[
    ".ollama",
    ".lmstudio",
    "lm studio",
    "huggingface",
    "comfyui",
    "stable-diffusion-webui",
    "whisper",
    ".cache\\huggingface",
];
const CACHE_DIRS: &[&str] = &[
    "temp",
    "tmp",
    "cache",
    "caches",
    "cache2",
    "_cacache",
    "code cache",
    "gpucache",
    "shadercache",
    "grshadercache",
    "dxcache",
    "glcache",
    "d3dscache",
    "inetcache",
    "crashdumps",
    "npm-cache",
    ".npm",
    "pnpm",
    "pnpm-cache",
    "yarn",
    ".yarn",
    "pip",
    "uv",
    "go-build",
    ".gradle",
    ".m2",
    ".nuget",
    "nuget",
    "ms-playwright",
    "cypress",
    ".cache",
    "logs",
    "htmlcache",
    "webcache",
    "cacheddata",
    "cachedextensionvsixs",
    "service worker",
    "delivery optimization",
    "softwaredistribution",
];
const PROJECT_DIRS: &[&str] = &[
    "node_modules",
    ".next",
    ".turbo",
    ".svelte-kit",
    ".nuxt",
    ".git",
    "__pycache__",
    ".venv",
    "venv",
    ".gradle-project",
    "source",
    "repos",
];
const MEDIA_DIRS: &[&str] = &[
    "documents",
    "pictures",
    "videos",
    "music",
    "downloads",
    "desktop",
    "onedrive",
    "camera roll",
    "screenshots",
    "photos",
    "movies",
];

/// Category for a folder named `name` (any case) whose parent has `parent` category. `depth` is
/// the depth below the drive root (1 = directly in `C:\`). `is_project` is true when the folder
/// holds a project marker file.
pub fn classify_dir(name: &str, parent: Category, depth: usize, is_project: bool) -> Category {
    let n = name.to_lowercase();
    if depth == 1 && SYSTEM_ROOT_DIRS.contains(&n.as_str()) {
        return Category::System;
    }
    if AI_DIRS.contains(&n.as_str()) {
        return Category::Ai;
    }
    if is_project || PROJECT_DIRS.contains(&n.as_str()) {
        return Category::Projects;
    }
    if CACHE_DIRS.contains(&n.as_str())
        || n.ends_with("cache")
        || n.ends_with("-cache")
        || n.ends_with("_cache")
    {
        // Caches inside a project are build output, which the Projects colour already explains.
        return if parent == Category::Projects {
            Category::Projects
        } else {
            Category::Caches
        };
    }
    if parent == Category::Projects
        || parent == Category::Ai
        || parent == Category::Caches
        || parent == Category::System
    {
        return parent;
    }
    if n == "appdata" {
        return Category::Apps;
    }
    if APP_DIRS.contains(&n.as_str()) {
        return Category::Apps;
    }
    if MEDIA_DIRS.contains(&n.as_str()) {
        return Category::Media;
    }
    parent
}

/// Category for a single large file.
pub fn classify_file(name: &str, parent: Category, depth: usize) -> Category {
    let n = name.to_lowercase();
    if depth == 1 && SYSTEM_ROOT_FILES.contains(&n.as_str()) {
        return Category::System;
    }
    let ext = n.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "gguf" | "safetensors" | "ckpt" | "onnx" | "pt" | "pth" | "ggml" => Category::Ai,
        "vhdx" | "vhd" | "vmdk" | "qcow2" => {
            if parent == Category::Other {
                Category::Apps
            } else {
                parent
            }
        }
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "jpg" | "jpeg" | "png"
        | "heic" | "raw" | "cr2" | "nef" | "arw" | "dng" | "psd" | "mp3" | "flac" | "wav"
        | "m4a" | "aac" | "pdf" | "docx" | "xlsx" | "pptx" | "doc" | "xls" | "ppt" => {
            if parent == Category::Projects
                || parent == Category::Caches
                || parent == Category::Apps
            {
                parent
            } else {
                Category::Media
            }
        }
        _ => parent,
    }
}

/// Files that mark a folder as a code project.
pub fn is_project_marker(file_name: &str) -> bool {
    let n = file_name.to_lowercase();
    matches!(
        n.as_str(),
        "package.json"
            | "cargo.toml"
            | "pyproject.toml"
            | "go.mod"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "composer.json"
            | "pubspec.yaml"
            | "mix.exs"
            | "build.zig"
            | "cmakelists.txt"
            | "package.swift"
    ) || n.ends_with(".sln")
        || n.ends_with(".csproj")
        || n.ends_with(".uproject")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherits_and_overrides() {
        assert_eq!(
            classify_dir("Windows", Category::Other, 1, false),
            Category::System
        );
        assert_eq!(
            classify_dir("Windows", Category::Other, 3, false),
            Category::Other
        );
        assert_eq!(
            classify_dir("Documents", Category::Other, 3, false),
            Category::Media
        );
        assert_eq!(
            classify_dir("app", Category::Media, 4, true),
            Category::Projects
        );
        assert_eq!(
            classify_dir("node_modules", Category::Media, 5, false),
            Category::Projects
        );
        assert_eq!(
            classify_dir("Cache", Category::Apps, 6, false),
            Category::Caches
        );
        assert_eq!(
            classify_dir(".cache", Category::Projects, 6, false),
            Category::Projects
        );
        assert_eq!(
            classify_dir(".ollama", Category::Other, 3, false),
            Category::Ai
        );
        assert_eq!(classify_dir("models", Category::Ai, 4, false), Category::Ai);
        assert_eq!(
            classify_file("pagefile.sys", Category::Other, 1),
            Category::System
        );
        assert_eq!(
            classify_file("model.gguf", Category::Other, 4),
            Category::Ai
        );
        assert_eq!(
            classify_file("movie.mkv", Category::Other, 4),
            Category::Media
        );
    }

    #[test]
    fn project_markers() {
        assert!(is_project_marker("package.json"));
        assert!(is_project_marker("App.sln"));
        assert!(!is_project_marker("readme.md"));
    }
}
