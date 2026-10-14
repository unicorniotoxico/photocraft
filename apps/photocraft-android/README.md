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
- **Screen fit**: the window spans the screen edge to edge and the UI keeps out of the status
  bar, navigation bar and camera cutout (Java reports their insets, egui's safe area).
- **Touch controls** (`photocraft_ui_egui::touch_controls`): floating buttons over the desktop UI
  (commands, keys, Ctrl/Shift/Alt held or latched, click, right click) and a cursor mode where the
  screen works as a trackpad with adjustable sensitivity. The gear button opens their settings:
  move, resize, add, remove and reassign buttons. Saved in `touch_controls.json` in the app's data.
- **Layout**: the desktop layout, portrait or landscape (rotation follows the system setting).
  `photocraft_ui_egui::mobile` has a compact phone layout (`CompactLayout::Auto`), off for now.
- Preferences and brush presets persist in the app's private storage.

## Not done yet

- Touch-sized controls inside the panels and dialogs (they keep their desktop sizes).
- OS clipboard, folder pickers, autosave/recovery, two-finger rotate.
- Text input relies on the soft keyboard sending key events (no IME composition).
