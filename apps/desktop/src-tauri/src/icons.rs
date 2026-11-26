//! The pictures shown in the launcher's rows: app icons, image files, images on the web and site favicons.
//!
//! An app's icon is the one Finder shows, which is not always a file in the bundle (many apps keep it in an asset
//! catalog), so it is drawn by the system. Images from the web are kept on disk, so the launcher does not go to the
//! network each time it is shown.

use crate::session::IconRef;
use flintkey_core::{favicon, icon};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Drawn at 2× the size the page shows it, for Retina displays.
#[cfg(target_os = "macos")]
const PIXELS: isize = 64;
/// Images larger than this are not shown (an icon is a few KB; this stops a wrong path from loading a photo).
const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// An image from the web older than this is fetched again; until the fetch succeeds the old one is shown.
const REFRESH_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// A picture that could not be loaded is not tried again for this long (the network may be down, or a file may be
/// added later).
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

enum Loaded {
    Image(Vec<u8>),
    Missing(Instant),
}

pub struct Icons {
    loaded: Mutex<HashMap<IconRef, Loaded>>,
    /// Icons are drawn off the main thread, which must stay free to deliver key presses. AppKit allows drawing
    /// into a bitmap on another thread (the current context is per thread), but `NSImage` is not documented as safe
    /// to use from several threads at once, so drawings take turns.
    drawing: Mutex<()>,
    /// Where images from the web are kept.
    cache_dir: PathBuf,
    config_dir: PathBuf,
}

impl Icons {
    pub fn new(cache_dir: PathBuf, config_dir: PathBuf) -> Self {
        Self {
            loaded: Mutex::new(HashMap::new()),
            drawing: Mutex::new(()),
            cache_dir,
            config_dir,
        }
    }

    /// The image bytes of `icon` (PNG, ICO, SVG, …), or `None` if it cannot be loaded. May go to the network, so
    /// call it off the main thread.
    pub fn image(&self, icon: &IconRef) -> Option<Vec<u8>> {
        match self.loaded.lock().unwrap().get(icon) {
            Some(Loaded::Image(bytes)) => return Some(bytes.clone()),
            Some(Loaded::Missing(when)) if when.elapsed() < RETRY_AFTER => return None,
            _ => {}
        }
        let image = self.load(icon);
        let entry = match &image {
            Some(bytes) => Loaded::Image(bytes.clone()),
            None => Loaded::Missing(Instant::now()),
        };
        self.loaded.lock().unwrap().insert(icon.clone(), entry);
        image
    }

    /// Forgets the images read from files and the pictures that could not be loaded, after the configuration
    /// changed: a file may have been replaced, or an icon value fixed. Images from the web stay (they are on disk).
    pub fn forget_local(&self) {
        self.loaded.lock().unwrap().retain(|icon, loaded| {
            matches!(icon, IconRef::Url { .. } | IconRef::Favicon { .. }) && matches!(loaded, Loaded::Image(_))
        });
    }

    fn load(&self, icon: &IconRef) -> Option<Vec<u8>> {
        match icon {
            IconRef::App { path } if path.ends_with(".app") => {
                let _drawing = self.drawing.lock().unwrap();
                draw(path)
            }
            IconRef::App { .. } => None,
            IconRef::File { path } => self.file(Path::new(path)),
            IconRef::Url { url } => cached(&self.cache_dir, &format!("url:{url}"), || {
                download(url, MAX_BYTES).filter(|bytes| is_image(bytes))
            }),
            IconRef::Favicon { url } => cached(&self.cache_dir, &format!("favicon:{}", without_fragment(url)), || {
                site_icon(url)
            }),
        }
    }

    fn file(&self, path: &Path) -> Option<Vec<u8>> {
        // A path written relative to the icons folder must stay in it, even through a link.
        let icons_dir = self.config_dir.join(icon::ICONS_DIR);
        if path.starts_with(&icons_dir) && !icon::stays_in_icons_dir(path, &self.config_dir) {
            return None;
        }
        if std::fs::metadata(path).ok()?.len() > MAX_BYTES {
            return None;
        }
        std::fs::read(path).ok().filter(|bytes| is_image(bytes))
    }
}

fn without_fragment(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// The image kept for `key` in `dir`, fetched again with `fetch` when missing or older than [`REFRESH_AFTER`]. A
/// failed fetch keeps the old image.
fn cached(dir: &Path, key: &str, fetch: impl FnOnce() -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let path = dir.join(cache_name(key));
    let kept = std::fs::read(&path).ok();
    let fresh = std::fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < REFRESH_AFTER);
    if fresh && kept.is_some() {
        return kept;
    }
    match fetch() {
        Some(bytes) => {
            let _ = std::fs::create_dir_all(dir);
            let partial = path.with_extension("partial");
            if std::fs::write(&partial, &bytes).is_ok() {
                let _ = std::fs::rename(&partial, &path);
            }
            Some(bytes)
        }
        None => kept,
    }
}

/// A file name for `key`. Not a cryptographic hash: a collision only shows another site's icon.
fn cache_name(key: &str) -> String {
    // FNV-1a, so the name stays the same across Rust versions (unlike `DefaultHasher`).
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}.img")
}

/// The icon of the site of `page_url`: the icons its HTML declares, best first, then `/favicon.ico`.
fn site_icon(page_url: &str) -> Option<Vec<u8>> {
    let html = download(page_url, MAX_BYTES).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    favicon::candidates(html.as_deref().unwrap_or(""), page_url)
        .iter()
        .find_map(|url| download(url, MAX_BYTES).filter(|bytes| is_image(bytes)))
}

/// GETs `url` with curl (on macOS and on Windows 10 and later it is part of the system), following redirects.
/// `None` on any failure, including an HTTP error status or a body over `max_bytes`.
fn download(url: &str, max_bytes: u64) -> Option<Vec<u8>> {
    let mut command = Command::new("curl");
    command.args([
        "--fail",
        "--silent",
        "--location",
        "--max-time",
        "5",
        "--max-filesize",
        &max_bytes.to_string(),
        "--proto",
        "=http,https",
        "--proto-redir",
        "=http,https",
        "--user-agent",
        "Mozilla/5.0 (compatible; Flintkey)",
        "--",
        url,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes up for each icon.
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok()?;
    let body = output.stdout;
    (output.status.success() && !body.is_empty() && body.len() as u64 <= max_bytes).then_some(body)
}

/// Whether `bytes` start like an image the page can show. A server answering an icon URL with an HTML error page
/// (with status 200) is common.
fn is_image(bytes: &[u8]) -> bool {
    const SIGNATURES: [&[u8]; 6] = [b"\x89PNG", b"\xff\xd8\xff", b"GIF8", b"\0\0\x01\0", b"BM", b"RIFF"];
    if SIGNATURES.iter().any(|signature| bytes.starts_with(signature)) {
        return !bytes.starts_with(b"RIFF") || bytes.get(8..12) == Some(b"WEBP");
    }
    let start = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let start = start.trim_start_matches('\u{feff}').trim_start();
    (start.starts_with("<svg") || start.starts_with("<?xml") || start.starts_with("<!--")) && start.contains("<svg")
}

#[cfg(target_os = "macos")]
fn draw(path: &str) -> Option<Vec<u8>> {
    use objc2::AnyThread;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSGraphicsContext,
        NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    autoreleasepool(|_| {
        let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
        // SAFETY: null planes make the rep allocate its own buffer; the other arguments describe 8-bit RGBA.
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                PIXELS,
                PIXELS,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                0,
                0,
            )
        }?;
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));
        let size = PIXELS as f64;
        image.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size, size)),
            // A zero rectangle means the whole image.
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.0,
        );
        NSGraphicsContext::restoreGraphicsState_class();
        // SAFETY: an empty dictionary is a valid set of properties.
        let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
        Some(data.to_vec())
    })
}

#[cfg(not(target_os = "macos"))]
fn draw(_path: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_image_formats_count_as_images() {
        // Arrange
        let images: [&[u8]; 6] = [
            b"\x89PNG\r\n\x1a\n....",
            b"\0\0\x01\0\x01\0",
            b"RIFF\0\0\0\0WEBPVP8 ",
            b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            b"  <svg viewBox=\"0 0 1 1\"></svg>",
            b"GIF89a",
        ];
        let not_images: [&[u8]; 4] = [
            b"<!doctype html><html>",
            b"",
            b"RIFF\0\0\0\0WAVEfmt ",
            b"{\"error\": 1}",
        ];

        // Act / Assert
        for bytes in images {
            assert!(is_image(bytes), "{:?}", String::from_utf8_lossy(bytes));
        }
        for bytes in not_images {
            assert!(!is_image(bytes), "{:?}", String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    fn a_kept_image_is_used_until_it_is_old_and_kept_when_refreshing_fails() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let fetched = std::cell::Cell::new(0);
        let fetch = |result: Option<&[u8]>| {
            fetched.set(fetched.get() + 1);
            result.map(<[u8]>::to_vec)
        };

        // Act
        let first = cached(dir.path(), "url:a", || fetch(Some(b"one")));
        let second = cached(dir.path(), "url:a", || fetch(Some(b"two")));
        let path = dir.path().join(cache_name("url:a"));
        let old = SystemTime::now() - REFRESH_AFTER - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let offline = cached(dir.path(), "url:a", || fetch(None));
        let refreshed = cached(dir.path(), "url:a", || fetch(Some(b"three")));
        let never = cached(dir.path(), "url:b", || fetch(None));

        // Assert
        assert_eq!(first.as_deref(), Some(&b"one"[..]));
        assert_eq!(
            second.as_deref(),
            Some(&b"one"[..]),
            "a fresh image is not fetched again"
        );
        assert_eq!(
            offline.as_deref(),
            Some(&b"one"[..]),
            "a failed refresh keeps the old image"
        );
        assert_eq!(refreshed.as_deref(), Some(&b"three"[..]));
        assert_eq!(never, None);
        assert_eq!(fetched.get(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_read_only_as_images_and_icons_folder_links_may_not_leave_it() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let icons_dir = config.join(icon::ICONS_DIR);
        std::fs::create_dir_all(&icons_dir).unwrap();
        std::fs::write(icons_dir.join("ok.png"), b"\x89PNG....").unwrap();
        std::fs::write(icons_dir.join("notes.png"), b"just text").unwrap();
        std::fs::write(dir.path().join("elsewhere.png"), b"\x89PNG....").unwrap();
        std::os::unix::fs::symlink(dir.path().join("elsewhere.png"), icons_dir.join("link.png")).unwrap();
        let icons = Icons::new(dir.path().join("cache"), config);
        let file = |path: PathBuf| {
            icons.image(&IconRef::File {
                path: path.display().to_string(),
            })
        };

        // Act / Assert
        assert!(file(icons_dir.join("ok.png")).is_some());
        assert!(file(icons_dir.join("notes.png")).is_none());
        assert!(file(icons_dir.join("link.png")).is_none());
        // A full path written in the config may point anywhere.
        assert!(file(dir.path().join("elsewhere.png")).is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn draws_a_system_app_icon_as_a_square_png() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let icons = Icons::new(dir.path().join("cache"), dir.path().join("config"));

        // Act
        let png = icons
            .image(&IconRef::App {
                path: "/System/Applications/Calculator.app".into(),
            })
            .expect("no icon");

        // Assert
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // The IHDR chunk: width and height, big-endian, right after the signature and the chunk header.
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), PIXELS as u32);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), PIXELS as u32);
    }
}
