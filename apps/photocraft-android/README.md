# PhotoCraft for Android

The same PhotoCraft app (engine + `photocraft-ui-egui`) on Android phones and tablets: a
`NativeActivity` whose whole window is drawn by eframe/wgpu (Vulkan, or GLES 3 as a fallback).

| Piece | Where |
|---|---|
| Entry point, JNI bridge (the crate's only `unsafe`) | `src/jni_bridge.rs` |
| Platform `Services` and the eframe runner | `src/shell.rs` |
| Java shell: file pickers, share/"open with", soft keyboard, links | `android/app/src/main/java/org/photocraft/android/MainActivity.java` |
| Manifest (landscape, no restart on rotation, intent filters) | `android/app/src/main/AndroidManifest.xml` |

## Build

```sh
apps/photocraft-android/build.sh
```

It runs `cargo ndk … build --profile android -p photocraft-android` (arm64, API 26+) into
`android/app/src/main/jniLibs`, then `gradle assembleRelease`. The APK is
`android/app/build/outputs/apk/release/app-release.apk`, signed with the debug key so it installs
directly (`adb install -r …`). Sign with your own key before distributing it.

## What works the Android way

- **Open**: File › Open shows the system picker; Java detaches each file's descriptor and Rust
  reads it (no copy through the Java heap). Files shared to PhotoCraft or opened with it arrive
  the same way.
- **Save / Export**: the bytes go to the app's cache, then the system "create document" picker
  asks where to put them.
- **Pen**: pressure comes through winit's touch force, read by `stylus.rs` as on Windows.
- **Pinch** zooms the canvas (egui's touch `zoom_delta`); two fingers drag it.
- **Phone layout**: portrait or landscape (rotation follows the system setting). On a phone, or a
  tablet in portrait, the UI switches to the compact layout (`photocraft_ui_egui::mobile`): menus
  in one row that scrolls sideways, Undo / Redo / Fit and the panel groups in a bottom bar, and
  each panel group in a drawer over the canvas. A tablet in landscape keeps the desktop layout.
- Preferences and brush presets persist in the app's private storage.

## Not done yet

- Touch-sized controls inside the panels and dialogs (they keep their desktop sizes).
- OS clipboard, folder pickers, autosave/recovery, two-finger rotate.
- Text input relies on the soft keyboard sending key events (no IME composition).
