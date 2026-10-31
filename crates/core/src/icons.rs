//! Icons and thumbnails from the Windows shell, returned as PNG data URLs.
//!
//! `IShellItemImageFactory` gives the same images Explorer shows: the real icon of an exe or
//! shortcut, and cached thumbnails for photos and videos. Results are cached per path and size
//! for the life of the process. Other systems only get small image files back as they are.

use std::collections::HashMap;
#[cfg(windows)]
use std::ffi::c_void;
use std::sync::OnceLock;

use base64::Engine;
use parking_lot::Mutex;
#[cfg(windows)]
use windows::Win32::Foundation::SIZE;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HGDIOBJ,
};
#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
#[cfg(windows)]
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY,
};
#[cfg(windows)]
use windows::core::PCWSTR;

#[cfg(windows)]
use crate::winstr::{from_wide, wide};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageKind {
    /// The file's icon (exe, dll, ico, shortcut, or the icon of its file type).
    Icon,
    /// A content thumbnail (photos, videos, PDFs); falls back to nothing if Windows has none.
    Thumb,
}

type CacheKey = (String, u32, ImageKind);

fn cache() -> &'static Mutex<HashMap<CacheKey, Option<String>>> {
    static CELL: OnceLock<Mutex<HashMap<CacheKey, Option<String>>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Expands `%SystemRoot%`-style variables the way Explorer does.
#[cfg(windows)]
pub fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let src = wide(s);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return s.to_string();
    }
    from_wide(&buf)
}

#[cfg(not(windows))]
pub fn expand_env(s: &str) -> String {
    s.to_string()
}

/// Turns a registry icon reference such as `"C:\App\app.exe",0` into an existing file path.
pub fn icon_file(reference: &str) -> Option<String> {
    let mut s = reference.trim().to_string();
    if let Some(rest) = s.strip_prefix('"')
        && let Some(end) = rest.find('"')
    {
        s = rest[..end].to_string();
    } else if let Some((path, index)) = s.rsplit_once(',')
        && index
            .trim()
            .trim_start_matches('-')
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        s = path.to_string();
    }
    let s = expand_env(s.trim().trim_matches('"'));
    std::path::Path::new(&s).is_file().then_some(s)
}

/// Reads a shell image as straight-alpha RGBA.
#[cfg(windows)]
fn shell_rgba(path: &str, size: u32, flags: SIIGBF) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let w = wide(path);
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let side = size as i32;
        let hbmp = factory.GetImage(SIZE { cx: side, cy: side }, flags).ok()?;
        let mut bm = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(hbmp.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut c_void),
        );
        let (width, height) = (bm.bmWidth, bm.bmHeight.abs());
        if got == 0 || width <= 0 || height <= 0 || width > 1024 || height > 1024 {
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height: top-down rows.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (width * height * 4) as usize];
        let hdc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            hdc,
            hbmp,
            0,
            height as u32,
            Some(px.as_mut_ptr() as *mut c_void),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(hdc);
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        if lines == 0 {
            return None;
        }
        Some((width as u32, height as u32, bgra_to_rgba(px)))
    }
}

/// Converts GDI's BGRA to RGBA. Icons usually arrive with premultiplied alpha; when every
/// colour channel is at most its alpha we treat the data as premultiplied and undo it, and
/// images without any alpha become opaque.
#[cfg(windows)]
fn bgra_to_rgba(mut px: Vec<u8>) -> Vec<u8> {
    let has_alpha = px.chunks(4).any(|p| p[3] != 0);
    let premultiplied = has_alpha
        && px
            .chunks(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
    for p in px.chunks_mut(4) {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        if !has_alpha {
            p.copy_from_slice(&[r, g, b, 255]);
        } else if premultiplied && a > 0 && a < 255 {
            let un =
                |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            p.copy_from_slice(&[un(r), un(g), un(b), a]);
        } else {
            p.copy_from_slice(&[r, g, b, a]);
        }
    }
    px
}

#[cfg(windows)]
fn png_data_url(width: u32, height: u32, rgba: &[u8]) -> Option<String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(out)
    ))
}

/// Small PNG/JPEG/ICO files (Store app logos) are their own icon; send them as they are
/// instead of asking the shell for the generic image-file icon.
fn image_file(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    let mime = if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        return None;
    };
    let md = std::fs::metadata(path).ok()?;
    if md.len() > 512 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn render(path: &str, size: u32, kind: ImageKind) -> Option<String> {
    if kind == ImageKind::Icon
        && let Some(url) = image_file(path)
    {
        return Some(url);
    }
    shell_image(path, size, kind)
}

#[cfg(windows)]
fn shell_image(path: &str, size: u32, kind: ImageKind) -> Option<String> {
    let flags = match kind {
        ImageKind::Icon => SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        ImageKind::Thumb => SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK,
    };
    let (w, h, rgba) = shell_rgba(path, size, flags)?;
    png_data_url(w, h, &rgba)
}

#[cfg(not(windows))]
fn shell_image(_path: &str, _size: u32, _kind: ImageKind) -> Option<String> {
    None
}

/// Images for many paths at once. Work is spread over a few single-threaded COM apartments,
/// which the shell image APIs expect.
pub fn images(paths: &[String], size: u32, kind: ImageKind) -> HashMap<String, Option<String>> {
    let size = size.clamp(16, 256);
    let mut out = HashMap::new();
    let mut todo = Vec::new();
    {
        let c = cache().lock();
        for p in paths {
            match c.get(&(p.clone(), size, kind)) {
                Some(v) => {
                    out.insert(p.clone(), v.clone());
                }
                None => todo.push(p.clone()),
            }
        }
    }
    todo.sort();
    todo.dedup();
    if todo.is_empty() {
        return out;
    }
    let workers = 4.min(todo.len());
    let chunk = todo.len().div_ceil(workers);
    let handles: Vec<_> = todo
        .chunks(chunk)
        .map(|part| {
            let part = part.to_vec();
            std::thread::spawn(move || {
                #[cfg(windows)]
                let inited = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
                let res: Vec<(String, Option<String>)> = part
                    .into_iter()
                    .map(|p| {
                        let real = expand_env(&p);
                        let img = if std::path::Path::new(&real).exists() {
                            render(&real, size, kind)
                        } else {
                            None
                        };
                        (p, img)
                    })
                    .collect();
                #[cfg(windows)]
                if inited {
                    unsafe { CoUninitialize() };
                }
                res
            })
        })
        .collect();
    let mut c = cache().lock();
    for h in handles {
        if let Ok(list) = h.join() {
            for (p, img) in list {
                c.insert((p.clone(), size, kind), img.clone());
                out.insert(p, img);
            }
        }
    }
    out
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn parses_icon_references() {
        let windir = std::env::var("WINDIR").unwrap();
        let notepad = format!(r"{windir}\System32\notepad.exe");
        assert_eq!(
            icon_file(&format!("{notepad},0")).as_deref(),
            Some(notepad.as_str())
        );
        assert_eq!(
            icon_file(&format!("\"{notepad}\",-2")).as_deref(),
            Some(notepad.as_str())
        );
        assert_eq!(
            icon_file(r"%SystemRoot%\System32\notepad.exe").map(|s| s.to_lowercase()),
            Some(notepad.to_lowercase())
        );
        assert!(icon_file(r"C:\does\not\exist.exe,0").is_none());
    }

    #[test]
    fn converts_premultiplied_pixels() {
        // Half-transparent white, premultiplied: 128,128,128,128 -> 255,255,255,128.
        let out = bgra_to_rgba(vec![128, 128, 128, 128, 0, 0, 255, 255]);
        assert_eq!(&out[..4], &[255, 255, 255, 128]);
        assert_eq!(&out[4..], &[255, 0, 0, 255]);
        // No alpha at all: opaque.
        assert_eq!(bgra_to_rgba(vec![1, 2, 3, 0]), vec![3, 2, 1, 255]);
    }

    #[test]
    fn renders_a_system_icon() {
        let windir = std::env::var("WINDIR").unwrap();
        let path = format!(r"{windir}\System32\notepad.exe");
        let map = images(std::slice::from_ref(&path), 32, ImageKind::Icon);
        let url = map
            .get(&path)
            .cloned()
            .flatten()
            .expect("notepad has an icon");
        assert!(url.starts_with("data:image/png;base64,"));
    }
}
