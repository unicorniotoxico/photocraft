//! The bridge between Rust and `MainActivity.java`: the native entry point, calls into the
//! activity (file pickers, soft keyboard, URLs), and the native methods Java calls back.
//!
//! The crate's one `unsafe` module (see Cargo.toml): JNI hands out raw pointers, entry points must
//! be `#[no_mangle]`, and picked files arrive as raw file descriptors. Every block says why it is
//! sound; everything else is safe code.
#![allow(unsafe_code)]

use std::fs::File;
use std::io::Read as _;
use std::os::fd::{FromRawFd as _, OwnedFd};
use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};

use jni::objects::{JClass, JIntArray, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jfloat, jint};
use jni::{JNIEnv, JavaVM};
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, Inbox};
use winit::platform::android::activity::AndroidApp;

/// The largest picked file read into memory: the engine's own bounded reads take over from there,
/// but a phone runs out of memory long before, so refuse clearly instead.
const MAX_FILE_BYTES: u64 = 2 << 30;

static ANDROID: OnceLock<AndroidApp> = OnceLock::new();
static REPAINT: OnceLock<egui::Context> = OnceLock::new();
static INBOX: OnceLock<Inbox> = OnceLock::new();
/// The open picker's reply and whether it asked for several files.
static PENDING_OPEN: Mutex<Option<(FileDialogReply, bool)>> = Mutex::new(None);
static SAVES: Mutex<Vec<SaveOutcome>> = Mutex::new(Vec::new());
static DISPLAY: Mutex<Option<Display>> = Mutex::new(None);

/// The screen as Java last reported it (`nativeDisplayChanged`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    /// Android's density: physical pixels per dp (1 dp = 1/160 inch).
    pub density: f32,
    /// The window's smallest side in dp (under 600: a phone).
    pub smallest_width_dp: i32,
    /// Pixels at each edge covered by the status bar, navigation bar or a camera cutout:
    /// left, top, right, bottom.
    pub insets_px: [f32; 4],
}

/// The latest screen report, once Java has sent one.
pub fn display() -> Option<Display> {
    *DISPLAY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a save handed to the system picker ended.
pub enum SaveOutcome {
    Saved(String),
    Cancelled,
    Failed(String),
}

/// The native activity's entry point (android-activity calls it on its own thread).
#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("PhotoCraft"));
    // Panics go to logcat too (stderr goes nowhere on Android).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
    }));
    crate::shell::run(app);
}

pub fn init(app: AndroidApp) {
    let _ = ANDROID.set(app);
}

pub fn set_repaint(ctx: egui::Context) {
    let _ = REPAINT.set(ctx);
}

fn repaint() {
    if let Some(ctx) = REPAINT.get() {
        ctx.request_repaint();
    }
}

/// Files that arrive without a picker waiting for them (shared to PhotoCraft, or opened with it).
pub fn inbox() -> Inbox {
    INBOX.get_or_init(Inbox::default).clone()
}

pub fn take_save_outcomes() -> Vec<SaveOutcome> {
    std::mem::take(&mut *SAVES.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Runs `f` with this thread attached to the JVM and the activity object.
fn with_activity<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Result<R, String> {
    let app = ANDROID.get().ok_or("the activity is not ready")?;
    // SAFETY: `vm_as_ptr` is the process's `JavaVM*`, valid for the life of the process; the
    // wrapper doesn't destroy the VM when dropped.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
    // SAFETY: `activity_as_ptr` is the activity's global reference, valid while the activity
    // lives (it outlives every frame); `JObject` only borrows it and never deletes it.
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread_permanently().map_err(|e| e.to_string())?;
    let result = f(&mut env, &activity);
    // A Java exception must not stay pending on this thread: log and clear it.
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result.map_err(|e| e.to_string())
}

/// File › Open: the system picker; the answer arrives through `nativeFilesPicked`.
pub fn pick_files(multiple: bool, reply: FileDialogReply) {
    // Only one picker at a time: a reply still waiting is answered as cancelled.
    if let Some((old, _)) = PENDING_OPEN.lock().unwrap_or_else(PoisonError::into_inner).replace((reply, multiple)) {
        old.send(None);
    }
    let shown = with_activity(|env, activity| env.call_method(activity, "pickFiles", "(Z)V", &[JValue::Bool(jboolean::from(multiple))]).map(|_| ()));
    if let Err(e) = shown {
        log::error!("couldn't show the file picker: {e}");
        if let Some((reply, _)) = PENDING_OPEN.lock().unwrap_or_else(PoisonError::into_inner).take() {
            reply.send(Some(FileDialogAnswer::Unavailable(format!("The system file picker could not be opened: {e}"))));
        }
    }
}

/// Writes `bytes` to the app's cache and asks the system "create document" picker where to put
/// them; Java copies the file there and reports through `nativeSaveFinished`.
pub fn save_file(outgoing: &Path, path: &str, bytes: &[u8]) -> Result<(), String> {
    let name = crate::display_name(path);
    std::fs::create_dir_all(outgoing).map_err(|e| format!("{}: {e}", outgoing.display()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let tmp = outgoing.join(format!("{stamp}-{name}"));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let Some(tmp_str) = tmp.to_str() else {
        let _ = std::fs::remove_file(&tmp);
        return Err("the app's cache path is not valid UTF-8".into());
    };
    let handed = with_activity(|env, activity| {
        let jname = env.new_string(&name)?;
        let jmime = env.new_string(crate::mime_for(&name))?;
        let jtmp = env.new_string(tmp_str)?;
        env.call_method(
            activity,
            "saveFile",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::Object(&jname), JValue::Object(&jmime), JValue::Object(&jtmp)],
        )
        .map(|_| ())
    });
    if handed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    handed
}

pub fn open_url(url: &str) -> Result<(), String> {
    with_activity(|env, activity| {
        let jurl = env.new_string(url)?;
        env.call_method(activity, "openUrl", "(Ljava/lang/String;)V", &[JValue::Object(&jurl)]).map(|_| ())
    })
}

pub fn set_keyboard_visible(visible: bool) {
    if let Err(e) = with_activity(|env, activity| env.call_method(activity, "setKeyboardVisible", "(Z)V", &[JValue::Bool(jboolean::from(visible))]).map(|_| ()))
    {
        log::warn!("couldn't toggle the soft keyboard: {e}");
    }
}

/// Java (a worker thread, never the UI thread): the user picked these files, or shared them to
/// PhotoCraft. Each `fds[i]` was detached from its `ParcelFileDescriptor`, so it is ours to close.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_photocraft_android_MainActivity_nativeFilesPicked<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    names: JObjectArray<'local>,
    fds: JIntArray<'local>,
) {
    let received = adopt(&mut env, &names, &fds);
    let mut files = Vec::with_capacity(received.len());
    let mut errors = Vec::new();
    for (name, fd) in received {
        match read_all(fd) {
            Ok(bytes) => files.push((name, bytes)),
            Err(e) => {
                log::error!("couldn't read {name}: {e}");
                errors.push(format!("{name}: {e}"));
            }
        }
    }
    deliver(files, errors);
}

/// Java: the picker was closed without choosing anything.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_photocraft_android_MainActivity_nativePickCancelled<'local>(_env: JNIEnv<'local>, _class: JClass<'local>) {
    if let Some((reply, _)) = PENDING_OPEN.lock().unwrap_or_else(PoisonError::into_inner).take() {
        reply.send(None);
    }
}

/// Java: a save handed to the system picker ended. `message` is the saved file's name on
/// success, the error on failure, and null when the user cancelled.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_photocraft_android_MainActivity_nativeSaveFinished<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    ok: jboolean,
    message: JString<'local>,
) {
    let text = if message.is_null() { None } else { env.get_string(&message).ok().map(String::from) };
    let outcome = match (ok != 0, text) {
        (true, name) => SaveOutcome::Saved(name.unwrap_or_default()),
        (false, None) => SaveOutcome::Cancelled,
        (false, Some(error)) => SaveOutcome::Failed(error),
    };
    SAVES.lock().unwrap_or_else(PoisonError::into_inner).push(outcome);
    repaint();
}

/// Java (UI thread): the density, the window's smallest width or the system bars / cutout insets
/// changed (start, rotation, bars shown or hidden).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_photocraft_android_MainActivity_nativeDisplayChanged<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    density: jfloat,
    smallest_width_dp: jint,
    left: jint,
    top: jint,
    right: jint,
    bottom: jint,
) {
    let px = |v: jint| v.max(0) as f32;
    let d = Display { density, smallest_width_dp, insets_px: [px(left), px(top), px(right), px(bottom)] };
    log::info!("display: {d:?}");
    *DISPLAY.lock().unwrap_or_else(PoisonError::into_inner) = Some(d);
    repaint();
}

/// Takes ownership of every descriptor first (so none leaks if a name can't be read), then pairs
/// each with its display name.
fn adopt(env: &mut JNIEnv, names: &JObjectArray, fds: &JIntArray) -> Vec<(String, OwnedFd)> {
    let count = env.get_array_length(fds).ok().and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let mut raw: Vec<jint> = vec![0; count];
    if count == 0 || env.get_int_array_region(fds, 0, &mut raw).is_err() {
        return Vec::new();
    }
    let owned: Vec<OwnedFd> = raw
        .into_iter()
        .filter(|fd| *fd >= 0)
        // SAFETY: Java called `ParcelFileDescriptor.detachFd()` for each of these and keeps no
        // copy, so nothing else closes them: this `OwnedFd` is their only owner.
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
        .collect();
    owned
        .into_iter()
        .enumerate()
        .map(|(i, fd)| {
            let name = jint::try_from(i)
                .ok()
                .and_then(|i| env.get_object_array_element(names, i).ok())
                .and_then(|o| env.get_string(&JString::from(o)).ok().map(String::from))
                .unwrap_or_else(|| "untitled".into());
            (name, fd)
        })
        .collect()
}

fn read_all(fd: OwnedFd) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::from(fd).take(MAX_FILE_BYTES.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(std::io::Error::other(format!("larger than {} GB, too large to open on this device", MAX_FILE_BYTES >> 30)));
    }
    Ok(bytes)
}

/// Answers the waiting picker, or puts the files in the inbox (shared or "open with" files).
fn deliver(mut files: Vec<(String, Vec<u8>)>, errors: Vec<String>) {
    let pending = PENDING_OPEN.lock().unwrap_or_else(PoisonError::into_inner).take();
    match pending {
        Some((reply, multiple)) => {
            let answer = if files.is_empty() {
                (!errors.is_empty()).then(|| FileDialogAnswer::Unavailable(format!("Could not read the file: {}", errors.join("; "))))
            } else if multiple {
                Some(FileDialogAnswer::ContentsMany(files))
            } else {
                let (name, bytes) = files.swap_remove(0);
                Some(FileDialogAnswer::Contents(name, bytes))
            };
            reply.send(answer);
        }
        None if !files.is_empty() => {
            inbox().lock().unwrap_or_else(PoisonError::into_inner).extend(files);
            repaint();
        }
        None => {}
    }
}
