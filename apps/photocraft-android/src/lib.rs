//! PhotoCraft on Android.
//!
//! Runs the same [`photocraft_ui_egui::PhotocraftApp`] as the desktop and web apps through
//! eframe's native runner on a `NativeActivity` (wgpu: Vulkan where available, GLES otherwise).
//! The Java side is `android/app/src/main/java/org/photocraft/android/MainActivity.java`; build
//! with `apps/photocraft-android/build.sh` (see this crate's README).
//!
//! Differences from the desktop app:
//! - no TCP control server, title bar, menu bar integration or OS clipboard yet;
//! - File › Open uses the system file picker (Storage Access Framework): Java hands over each
//!   picked file as a file descriptor, read here into the answer the app expects (as on the web,
//!   contents rather than paths). Files shared to PhotoCraft or opened with it from another app
//!   arrive the same way, through `Services::inbox`;
//! - saving writes the bytes to the app's cache and asks the system "create document" picker
//!   where to put them (the web build's download, on Android);
//! - preferences and brush presets live in the app's private data directory;
//! - pen pressure needs no extra code: winit reports `MotionEvent` pressure as touch force, which
//!   `photocraft_ui_egui::stylus` reads as it does on Windows.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "android")]
mod jni_bridge;
#[cfg(target_os = "android")]
mod shell;

/// The MIME type the system "create document" picker offers for a file name.
pub fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("tif" | "tiff") => "image/tiff",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("psd" | "psb") => "image/vnd.adobe.photoshop",
        Some("ora") => "image/openraster",
        Some("kra" | "krz") => "application/x-krita",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Pixels per point for the UI: Android's density (so 1 point = 1 dp, the size Android apps are
/// designed in), 15 % larger on phones (smallest side under 600 dp), whose controls are tapped
/// with a finger rather than a pen. `None` for a density Java couldn't read.
pub fn ui_scale(density: f32, smallest_width_dp: i32) -> Option<f32> {
    if !density.is_finite() || density <= 0.0 {
        return None;
    }
    let phone = smallest_width_dp > 0 && smallest_width_dp < 600;
    Some(if phone { density * 1.15 } else { density })
}

/// The last component of a path or suggested name: what the save picker offers as the file name.
pub fn display_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_follow_the_extension() {
        assert_eq!(mime_for("photo.JPG"), "image/jpeg");
        assert_eq!(mime_for("art.psd"), "image/vnd.adobe.photoshop");
        assert_eq!(mime_for("doc.pcraft"), "application/octet-stream");
        assert_eq!(mime_for("no-extension"), "application/octet-stream");
    }

    #[test]
    fn ui_scale_follows_the_density_and_grows_on_phones() {
        assert_eq!(ui_scale(2.0, 800), Some(2.0));
        assert!(ui_scale(2.75, 392).is_some_and(|s| (s - 2.75 * 1.15).abs() < 1e-5));
        assert_eq!(ui_scale(0.0, 392), None);
        assert_eq!(ui_scale(f32::NAN, 392), None);
        // An unknown width counts as a tablet: no boost.
        assert_eq!(ui_scale(3.0, 0), Some(3.0));
    }

    #[test]
    fn display_name_keeps_only_the_file_name() {
        assert_eq!(display_name("/storage/emulated/0/Pictures/a.png"), "a.png");
        assert_eq!(display_name("b.psd"), "b.psd");
        assert_eq!(display_name(""), "");
    }
}
