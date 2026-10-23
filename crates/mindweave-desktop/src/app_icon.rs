//! The app's icon. While the app runs it is set as the Dock icon (macOS);
//! a packaged app also carries it in its bundle (`assets/app-icon.icns`).

/// The icon, a 1024 × 1024 PNG with a transparent margin (the source
/// drawing is `assets/app-icon.svg`).
// Shown only by macOS for now (other systems take the icon from the
// package); its test runs everywhere.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

/// Shows [`APP_ICON_PNG`] as the app's Dock icon. Does nothing off the main
/// thread or when the picture does not decode.
#[cfg(target_os = "macos")]
pub fn set_dock_icon() {
    use objc2::{AllocAnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(APP_ICON_PNG);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(main_thread);
    // SAFETY: on the main thread, with a valid image.
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

/// Other platforms take the icon from the package.
#[cfg(not(target_os = "macos"))]
pub fn set_dock_icon() {}

#[cfg(test)]
mod tests {
    use super::APP_ICON_PNG;

    #[test]
    fn the_icon_is_a_png() {
        assert!(APP_ICON_PNG.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
