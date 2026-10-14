//! The Android shell: `Services` for the app, and the eframe runner on the activity.

use std::path::{Path, PathBuf};

use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::touch_controls::{Config, TouchControls};
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, PhotocraftApp, Services, i18n};
use winit::platform::android::activity::AndroidApp;

use crate::jni_bridge;

pub fn run(android: AndroidApp) {
    let data_dir = android.internal_data_path();
    if data_dir.is_none() {
        log::warn!("no internal data path: preferences and brush presets are session-only");
    }
    jni_bridge::init(android.clone());

    let mut options = eframe::NativeOptions { android_app: Some(android), renderer: eframe::Renderer::Wgpu, ..Default::default() };
    // The adapter's real texture limits (egui asks for 8192 px), so big documents stay on the GPU
    // where the phone's GPU allows it.
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);

    // Brush presets load in the background; the app attaches them when they arrive.
    let presets = data_dir.as_ref().map(|d| photocraft_engine::preset_store::open_dir_async(d.join("presets")));
    let touch_file = data_dir.as_ref().map(|d| d.join("touch_controls.json"));
    let touch = TouchControls::new(touch_file.as_deref().and_then(load_touch_config).unwrap_or_default());

    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
            let mut services = services(data_dir.as_deref());
            services.preset_store = presets;
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            app.set_theme(&cc.egui_ctx, ThemeKind::Pro);
            // Threads work as on the desktop: long commands and file opens run as background jobs
            // with progress and Cancel, so the UI keeps answering (and Android doesn't report it
            // as not responding).
            app.background_jobs = true;
            if let Some(rs) = cc.wgpu_render_state.clone() {
                let info = rs.adapter.get_info();
                log::info!("PhotoCraft: wgpu {:?} on {} ({:?})", info.backend, info.name, info.device_type);
                app.perf.gpu_info.set_adapter(&info);
                if info.device_type != eframe::wgpu::DeviceType::Cpu {
                    app.set_wgpu(rs);
                } else {
                    // The window still draws with wgpu: record its errors instead of panicking.
                    let _ = photocraft_ui_egui::gpu_canvas::DeviceHealth::watch(&rs.device);
                }
            }
            jni_bridge::set_repaint(cc.egui_ctx.clone());
            Ok(Box::new(AndroidShell { app, keyboard_visible: false, touch, touch_file }))
        }),
    );
    if let Err(e) = result {
        log::error!("PhotoCraft failed to start: {e}");
    }
}

struct AndroidShell {
    app: PhotocraftApp,
    /// What the soft keyboard was last asked to do, so Java is called only when it changes.
    keyboard_visible: bool,
    /// The on-screen buttons and the cursor mode, and where their settings are saved.
    touch: TouchControls,
    touch_file: Option<PathBuf>,
}

impl AndroidShell {
    /// Saves that finished (or failed) in the system picker since the last frame, as notices.
    fn report_saves(&mut self) {
        for outcome in jni_bridge::take_save_outcomes() {
            match outcome {
                jni_bridge::SaveOutcome::Saved(name) => log::info!("saved {name}"),
                jni_bridge::SaveOutcome::Cancelled => {}
                jni_bridge::SaveOutcome::Failed(error) => {
                    photocraft_ui_egui::notices::post(&mut self.app, i18n::t("Could not save the file"), vec![error], true, None);
                }
            }
        }
    }

    /// Shows the soft keyboard while a text field has focus, and hides it after.
    fn sync_keyboard(&mut self, ctx: &egui::Context) {
        // A focused text field, not any focused widget: a tapped button must not bring it up.
        let wanted = ctx.text_edit_focused();
        if wanted != self.keyboard_visible {
            self.keyboard_visible = wanted;
            jni_bridge::set_keyboard_visible(wanted);
        }
    }
}

impl eframe::App for AndroidShell {
    /// The status bar, navigation bar and camera cutout as egui's safe area: windows and popups
    /// stay inside it, and `ui` lays the app out in it.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(d) = jni_bridge::display() {
            let ppp = ctx.pixels_per_point();
            let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
            let [left, top, right, bottom] = d.insets_px.map(|v| v / ppp);
            raw_input.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { left, right, top, bottom }));
        }
        self.touch.process_input(ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.report_saves();
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // The root Ui spans the whole screen, safe area or not: lay the app out in the part the
        // system bars and the cutout leave free, so nothing is drawn under them.
        let safe = ui.ctx().content_rect();
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(safe));
        inner.set_clip_rect(safe);
        self.touch.run_queued(&mut self.app, ui.ctx());
        self.app.ui(&mut inner, frame);
        self.touch.show(ui.ctx());
        if self.touch.take_dirty(ui.ctx())
            && let Some(path) = &self.touch_file
        {
            match serde_json::to_string_pretty(&self.touch.config) {
                Ok(text) => {
                    if let Err(e) = write_atomically(path, text.as_bytes()) {
                        log::warn!("couldn't save the touch controls: {e}");
                    }
                }
                Err(e) => log::warn!("couldn't encode the touch controls: {e}"),
            }
        }
        self.sync_keyboard(ui.ctx());
    }
}

fn services(data_dir: Option<&Path>) -> Services {
    let prefs_file = data_dir.map(|d| d.join("preferences.json"));
    let prefs_to_save = prefs_file.clone();
    let outgoing = data_dir.map_or_else(|| std::env::temp_dir().join("photocraft-outgoing"), |d| d.join("outgoing"));
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], max_svg_group_depth: usize| {
            photocraft_io::import_with_svg_group_depth(name, bytes, max_svg_group_depth).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let mut opts = photocraft_io::ExportOptions::default();
            if let Some(q) = settings.jpeg_quality {
                opts.encode.jpeg_quality = q;
            }
            opts.encode.webp_lossless = settings.webp_lossless;
            if let Some(q) = settings.webp_quality {
                opts.encode.webp_quality = q;
            }
            opts.tiff_layers = settings.tiff_layers;
            opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
            opts.tga_bits = settings.tga_bits;
            opts.image_previews = settings.image_previews;
            opts.psd_maximize_compatibility = settings.psd_maximize_compatibility;
            photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        file_dialog: Some(Box::new(|request, _frame, reply| match request {
            // The system picker hands over documents, not paths: the answer carries contents.
            FileDialogRequest::Open { multiple, .. } => jni_bridge::pick_files(multiple, reply),
            // Storage Access Framework trees can't be engine paths.
            FileDialogRequest::Folder { .. } => reply.send(None),
            // The suggested name goes to the "create document" picker when the bytes are written.
            FileDialogRequest::Save { suggested } => reply.send(Some(FileDialogAnswer::SaveTo(crate::display_name(&suggested)))),
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| jni_bridge::save_file(&outgoing, path, bytes))),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        open_url: Some(Box::new(jni_bridge::open_url)),
        inbox: Some(jni_bridge::inbox()),
        load_prefs: Some(Box::new(move || prefs_file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()))),
        save_prefs: Some(Box::new(move |text: &str| match &prefs_to_save {
            Some(path) => write_atomically(path, text.as_bytes()),
            None => Err("no app data directory".into()),
        })),
        ..Default::default()
    }
}

/// The saved touch controls, if the file exists and reads.
fn load_touch_config(path: &Path) -> Option<Config> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(c) => Some(c),
        Err(e) => {
            log::warn!("{}: {e}; using the default touch controls", path.display());
            None
        }
    }
}

/// Write through a temporary file and rename it over `path`, so a crash mid-write never leaves a
/// truncated preferences file.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp: PathBuf = path.with_extension("tmp");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}
