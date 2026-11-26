//! App icons for the launcher's rows, as PNG.
//!
//! The icon is the one Finder shows, which is not always a file in the bundle (many apps keep it in an asset
//! catalog), so it is drawn by the system rather than read from disk.

use std::collections::HashMap;
use std::sync::Mutex;

/// Drawn at 2× the size the page shows it, for Retina displays.
#[cfg(target_os = "macos")]
const PIXELS: isize = 64;

/// PNGs by bundle path. An icon rarely changes while the app runs, and a list of results asks for the same few
/// icons on every keystroke.
#[derive(Default)]
pub struct IconCache {
    pngs: Mutex<HashMap<String, Option<Vec<u8>>>>,
    /// Icons are drawn off the main thread, which must stay free to deliver key presses. AppKit allows drawing
    /// into a bitmap on another thread (the current context is per thread), but `NSImage` is not documented as safe
    /// to use from several threads at once, so drawings take turns.
    drawing: Mutex<()>,
}

impl IconCache {
    pub fn png(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(cached) = self.pngs.lock().unwrap().get(path) {
            return cached.clone();
        }
        let png = {
            let _drawing = self.drawing.lock().unwrap();
            draw(path)
        };
        self.pngs.lock().unwrap().insert(path.to_string(), png.clone());
        png
    }
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn draws_a_system_app_icon_as_a_square_png() {
        // Arrange
        let cache = IconCache::default();

        // Act
        let png = cache.png("/System/Applications/Calculator.app").expect("no icon");

        // Assert
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // The IHDR chunk: width and height, big-endian, right after the signature and the chunk header.
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), PIXELS as u32);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), PIXELS as u32);
        assert!(
            cache
                .pngs
                .lock()
                .unwrap()
                .contains_key("/System/Applications/Calculator.app")
        );
    }
}
