//! On-screen touch controls for phones and tablets, in the spirit of game launchers' overlays:
//! the desktop UI stays as it is, and floating buttons the user places, sizes and assigns
//! provide what a touch screen lacks (modifier keys, keys, right click, common commands).
//!
//! - **Buttons** run a command, press a key, hold a modifier (or latch it with *toggle*), click,
//!   right-click, switch the cursor mode or open the settings. Touches on a button never reach
//!   the UI underneath.
//! - **Cursor mode** turns the screen into a trackpad: one finger moves an on-screen cursor by
//!   its movement × the sensitivity; a tap clicks; tap, then touch again and drag, drags. Two
//!   fingers still pinch-zoom and pan the canvas. While the settings window is open, or the
//!   buttons are being moved, touches act directly again.
//! - **Settings** (the gear button): show/hide, opacity, cursor mode and sensitivity, move the
//!   buttons by dragging, and add, change, resize or remove buttons.
//!
//! The shell calls [`TouchControls::process_input`] from `raw_input_hook` (before egui sees the
//! frame's input), [`TouchControls::run_queued`] before the app's UI and [`TouchControls::show`]
//! after it, and saves [`TouchControls::config`] when [`TouchControls::take_dirty`] says so.
//!
//! Strings are Spanish or English here rather than `tl!` catalog entries: this overlay is part of
//! the Android build only.

use std::collections::{HashMap, HashSet};

use egui::{Color32, Context, Event, Id, Key, Modifiers, Order, PointerButton, Pos2, Rect, Sense, Stroke, TouchPhase, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PhotocraftApp;

/// A modifier key a button holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
}

/// What a button does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// Run a command (an id from `menu_catalog`) when pressed.
    Command { id: String },
    /// Press a key while held (an egui key name: `Space`, `Escape`, `OpenBracket`, `B`…).
    Key { key: String },
    /// Hold a modifier while held.
    Modifier { modifier: Modifier },
    /// The primary button while held, at the cursor (or the last touch).
    LeftClick,
    /// A right click at the cursor (or the last touch).
    RightClick,
    /// Switch the cursor (trackpad) mode.
    ToggleCursor,
    /// Open or close the settings window.
    Settings,
}

impl Action {
    /// Whether *toggle* (latch on tap) makes sense for it.
    fn can_toggle(&self) -> bool {
        matches!(self, Action::Key { .. } | Action::Modifier { .. } | Action::LeftClick)
    }
}

/// One on-screen button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    pub action: Action,
    /// Text drawn on it when it has no icon.
    pub label: String,
    /// An icon from `assets/icons`, drawn instead of the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Centre, as fractions of the usable screen (0..1).
    pub x: f32,
    pub y: f32,
    /// Diameter in points.
    pub size: f32,
    /// A tap latches it on, the next tap releases it (keys, modifiers, click).
    #[serde(default)]
    pub toggle: bool,
}

/// Everything the user sets; saved by the shell as JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Show the buttons (the gear always shows, so they can be brought back).
    pub visible: bool,
    /// Button opacity, 0.1..1.
    pub opacity: f32,
    /// One finger moves a cursor instead of touching directly.
    pub cursor_mode: bool,
    /// Cursor movement per point of finger movement.
    pub sensitivity: f32,
    pub buttons: Vec<Button>,
}

impl Default for Config {
    fn default() -> Self {
        let place = |p: Preset, x: f32, y: f32| p.button(x, y);
        let presets = presets();
        let find = |key: &str| presets.iter().find(|p| p.key == key).cloned();
        let layout: [(&str, f32, f32); 11] = [
            ("settings", 0.78, 0.30),
            ("undo", 0.92, 0.30),
            ("redo", 0.92, 0.39),
            ("space", 0.78, 0.39),
            ("cursor", 0.92, 0.48),
            ("ctrl", 0.78, 0.48),
            ("click", 0.92, 0.57),
            ("shift", 0.78, 0.57),
            ("rightClick", 0.92, 0.66),
            ("alt", 0.78, 0.66),
            ("fit", 0.92, 0.75),
        ];
        Self {
            visible: true,
            opacity: 0.6,
            cursor_mode: false,
            sensitivity: 1.5,
            buttons: layout.iter().filter_map(|(k, x, y)| find(k).map(|p| place(p, *x, *y))).collect(),
        }
    }
}

impl Config {
    /// Repairs values a hand-edited or old file may hold, and makes sure a gear exists.
    pub fn sanitized(mut self) -> Self {
        let fix = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.opacity = fix(self.opacity, 0.1, 1.0, 0.6);
        self.sensitivity = fix(self.sensitivity, 0.2, 5.0, 1.5);
        for b in &mut self.buttons {
            b.x = fix(b.x, 0.0, 1.0, 0.5);
            b.y = fix(b.y, 0.0, 1.0, 0.5);
            b.size = fix(b.size, 28.0, 120.0, 52.0);
            if !b.action.can_toggle() {
                b.toggle = false;
            }
        }
        if !self.buttons.iter().any(|b| b.action == Action::Settings)
            && let Some(p) = presets().into_iter().find(|p| p.key == "settings")
        {
            self.buttons.push(p.button(0.78, 0.30));
        }
        self
    }
}

/// A button kind offered in the settings' "Add" and "Action" lists.
#[derive(Clone, Debug)]
pub struct Preset {
    pub key: &'static str,
    /// What the list shows (in the UI language).
    pub name: &'static str,
    /// What the button shows when it has no icon.
    pub short: &'static str,
    pub icon: Option<&'static str>,
    pub action: Action,
    pub toggle: bool,
}

impl Preset {
    fn button(&self, x: f32, y: f32) -> Button {
        Button { action: self.action.clone(), label: self.short.to_string(), icon: self.icon.map(str::to_string), x, y, size: 52.0, toggle: self.toggle }
    }
}

fn es() -> bool {
    crate::i18n::current().code().starts_with("es")
}

/// `en` or `es` by the UI language.
fn l(en: &'static str, es_: &'static str) -> &'static str {
    if es() { es_ } else { en }
}

/// Every button kind, in list order.
pub fn presets() -> Vec<Preset> {
    let cmd = |id: &str| Action::Command { id: id.to_string() };
    let key = |k: &str| Action::Key { key: k.to_string() };
    let p = |key_: &'static str, name: &'static str, short: &'static str, icon: Option<&'static str>, action: Action| Preset {
        key: key_,
        name,
        short,
        icon,
        action,
        toggle: false,
    };
    vec![
        p("settings", l("Touch controls settings", "Ajustes de controles"), "⚙", Some("settings"), Action::Settings),
        p("cursor", l("Cursor mode on/off", "Modo cursor sí/no"), "Cur", Some("mouse-pointer-2"), Action::ToggleCursor),
        p("click", l("Click (hold to drag)", "Clic (mantener para arrastrar)"), l("Click", "Clic"), None, Action::LeftClick),
        p("rightClick", l("Right click", "Clic derecho"), l("R-click", "Clic d."), None, Action::RightClick),
        p("ctrl", "Ctrl", "Ctrl", None, Action::Modifier { modifier: Modifier::Ctrl }),
        p("shift", "Shift", "Shift", None, Action::Modifier { modifier: Modifier::Shift }),
        p("alt", "Alt", "Alt", None, Action::Modifier { modifier: Modifier::Alt }),
        p("space", l("Space (Hand while held)", "Espacio (Mano mientras se mantiene)"), l("Space", "Esp."), Some("hand"), key("Space")),
        p("escape", "Esc", "Esc", None, key("Escape")),
        p("enter", "Enter", "Enter", None, key("Enter")),
        p("brushDown", l("[ (smaller brush)", "[ (pincel más chico)"), "[", None, key("OpenBracket")),
        p("brushUp", l("] (bigger brush)", "] (pincel más grande)"), "]", None, key("CloseBracket")),
        p("keyB", l("B (Brush)", "B (Pincel)"), "B", None, key("B")),
        p("keyE", l("E (Eraser)", "E (Borrador)"), "E", None, key("E")),
        p("keyV", l("V (Move)", "V (Mover)"), "V", None, key("V")),
        p("keyM", l("M (Marquee)", "M (Marco)"), "M", None, key("M")),
        p("keyL", l("L (Lasso)", "L (Lazo)"), "L", None, key("L")),
        p("undo", l("Undo", "Deshacer"), l("Undo", "Desh."), Some("undo-2"), cmd("edit.undo")),
        p("redo", l("Redo", "Rehacer"), l("Redo", "Reh."), Some("redo-2"), cmd("edit.redo")),
        p("fit", l("Fit on Screen", "Encajar en pantalla"), l("Fit", "Encajar"), Some("maximize-2"), cmd("view.fitOnScreen")),
        p("zoomIn", l("Zoom In", "Acercar"), "Z+", Some("zoom-in"), cmd("view.zoomIn")),
        p("zoomOut", l("Zoom Out", "Alejar"), "Z−", None, cmd("view.zoomOut")),
        p("copy", l("Copy", "Copiar"), l("Copy", "Copiar"), Some("copy"), cmd("edit.copy")),
        p("cut", l("Cut", "Cortar"), l("Cut", "Cortar"), None, cmd("edit.cut")),
        p("paste", l("Paste", "Pegar"), l("Paste", "Pegar"), None, cmd("edit.paste")),
        p("selectAll", l("Select All", "Seleccionar todo"), l("All", "Todo"), None, cmd("select.all")),
        p("deselect", l("Deselect", "Deseleccionar"), l("Desel.", "Desel."), None, cmd("select.deselect")),
        p("transform", l("Free Transform", "Transformación libre"), "Transf.", None, cmd("edit.freeTransform")),
        p("newLayer", l("New Layer", "Capa nueva"), l("+Layer", "+Capa"), None, cmd("layer.new.layer")),
        p("save", l("Save", "Guardar"), l("Save", "Guardar"), None, cmd("file.save")),
        p("open", l("Open", "Abrir"), l("Open", "Abrir"), None, cmd("file.open")),
    ]
}

/// What a touch is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// Pressing button `n`.
    Button(usize),
    /// Moving the cursor (cursor mode).
    Trackpad,
    /// A second finger in cursor mode (pinch, two-finger pan).
    Gesture,
    /// A direct touch, passed on as is.
    Direct,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    kind: Kind,
    start: Pos2,
    last: Pos2,
    t0: f64,
    /// Farthest it went from `start` (points).
    travel: f32,
}

/// A tap (cursor mode) shorter than this (seconds) and…
const TAP_TIME: f64 = 0.28;
/// …moving less than this (points) clicks.
const TAP_SLOP: f32 = 12.0;
/// A touch this soon after a tap, near it, starts a drag.
const DOUBLE_TAP: f64 = 0.35;

/// The overlay's state: the saved config plus what fingers are doing now.
#[derive(Default)]
pub struct TouchControls {
    pub config: Config,
    dirty: bool,
    settings_open: bool,
    /// Moving buttons by dragging (from the settings).
    editing: bool,
    /// The cursor position in cursor mode.
    cursor: Option<Pos2>,
    /// The last direct touch, where Click / Right click act outside cursor mode.
    last_touch: Option<Pos2>,
    touches: HashMap<u64, Touch>,
    /// Mirrors egui-winit: the touch it turns into pointer events (the first one down).
    pointer_touch: Option<u64>,
    /// The touch moving the cursor, and whether a second finger made it a gesture.
    trackpad: Option<u64>,
    gesture: bool,
    /// The primary button is down from a tap-and-drag or the Click button.
    left_down: bool,
    tap_drag: bool,
    last_tap: Option<(f64, Pos2)>,
    /// Buttons pressed by a finger now, and toggles latched on.
    held: HashSet<usize>,
    latched: HashSet<usize>,
    /// Commands to run before the next UI pass.
    queued: Vec<String>,
    /// Events for the next frame (a release right after a press).
    pending: Vec<Event>,
    /// The add-button list's choice in the settings.
    add_choice: usize,
}

impl TouchControls {
    pub fn new(config: Config) -> Self {
        Self { config: config.sanitized(), ..Default::default() }
    }

    /// The config changed and nothing is being dragged: time to save it.
    pub fn take_dirty(&mut self, ctx: &Context) -> bool {
        if self.dirty && !ctx.input(|i| i.pointer.any_down() || i.any_touches()) {
            self.dirty = false;
            return true;
        }
        false
    }

    /// Touches act directly while the settings are open or the buttons are being moved.
    fn cursor_active(&self) -> bool {
        self.config.cursor_mode && !self.settings_open && !self.editing
    }

    fn shown(&self, b: &Button) -> bool {
        self.config.visible || b.action == Action::Settings
    }

    /// Each shown button's index and rect on `screen`.
    fn rects(&self, screen: Rect) -> Vec<(usize, Rect)> {
        self.config
            .buttons
            .iter()
            .enumerate()
            .filter(|(_, b)| self.shown(b))
            .map(|(i, b)| (i, Rect::from_center_size(pos2(screen.left() + b.x * screen.width(), screen.top() + b.y * screen.height()), Vec2::splat(b.size))))
            .collect()
    }

    /// Held and latched modifiers.
    fn modifiers(&self) -> Modifiers {
        let mut m = Modifiers::NONE;
        for i in self.held.iter().chain(self.latched.iter()) {
            if let Some(Action::Modifier { modifier }) = self.config.buttons.get(*i).map(|b| &b.action) {
                match modifier {
                    Modifier::Ctrl => {
                        m.ctrl = true;
                        m.command = true;
                    }
                    Modifier::Shift => m.shift = true,
                    Modifier::Alt => m.alt = true,
                }
            }
        }
        m
    }

    /// Where Click and Right click act: the cursor in cursor mode, else the last direct touch.
    fn target(&self) -> Option<Pos2> {
        if self.cursor_active() { self.cursor } else { self.last_touch }
    }

    /// Rewrites this frame's input: touches on buttons become their actions, and in cursor mode
    /// one finger drives the cursor. Call from the shell's `raw_input_hook`.
    pub fn process_input(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        let screen = ctx.content_rect();
        let now = raw.time.unwrap_or_else(|| ctx.input(|i| i.time));
        if self.process(screen, now, raw) {
            ctx.request_repaint();
        }
    }

    /// [`Self::process_input`] without a context; returns whether another frame is needed.
    fn process(&mut self, screen: Rect, now: f64, raw: &mut egui::RawInput) -> bool {
        let rects = if self.editing { Vec::new() } else { self.rects(screen) };
        let mut out: Vec<Event> = std::mem::take(&mut self.pending);
        let mut skip_pointer = false;
        for ev in std::mem::take(&mut raw.events) {
            match ev {
                Event::Touch { id, phase, pos, .. } => {
                    let tid = id.0;
                    let is_pointer = match phase {
                        TouchPhase::Start => self.pointer_touch.is_none(),
                        _ => self.pointer_touch == Some(tid),
                    };
                    if phase == TouchPhase::Start && self.pointer_touch.is_none() {
                        self.pointer_touch = Some(tid);
                    }
                    if matches!(phase, TouchPhase::End | TouchPhase::Cancel) && self.pointer_touch == Some(tid) {
                        self.pointer_touch = None;
                    }
                    if phase == TouchPhase::Start {
                        let kind = match rects.iter().find(|(_, r)| r.contains(pos)) {
                            Some((i, _)) => Kind::Button(*i),
                            None if self.cursor_active() => {
                                if self.trackpad.is_some() {
                                    Kind::Gesture
                                } else {
                                    Kind::Trackpad
                                }
                            }
                            None => Kind::Direct,
                        };
                        self.touches.insert(tid, Touch { kind, start: pos, last: pos, t0: now, travel: 0.0 });
                        match kind {
                            Kind::Button(i) => self.press(i, &mut out),
                            Kind::Trackpad => self.trackpad_start(tid, pos, now, screen, &mut out),
                            Kind::Gesture => self.gesture = true,
                            Kind::Direct => {}
                        }
                    }
                    let Some(mut t) = self.touches.get(&tid).copied() else {
                        // A touch that began before this overlay saw it: pass it on.
                        skip_pointer = false;
                        out.push(ev);
                        continue;
                    };
                    t.travel = t.travel.max((pos - t.start).length());
                    let delta = pos - t.last;
                    t.last = pos;
                    let ended = matches!(phase, TouchPhase::End | TouchPhase::Cancel);
                    if ended {
                        self.touches.remove(&tid);
                    } else {
                        self.touches.insert(tid, t);
                    }
                    match t.kind {
                        Kind::Button(i) => {
                            if ended {
                                self.release(i, &mut out);
                            }
                            // Neither the touch nor the pointer it drives reaches the UI.
                            skip_pointer = is_pointer;
                            continue;
                        }
                        Kind::Trackpad => {
                            if phase == TouchPhase::Move && !self.gesture {
                                self.move_cursor(delta, screen, &mut out);
                            }
                            if ended {
                                self.trackpad_end(&t, phase, now, &mut out);
                            }
                            skip_pointer = is_pointer;
                        }
                        Kind::Gesture => {
                            skip_pointer = is_pointer;
                        }
                        Kind::Direct => {
                            if !ended {
                                self.last_touch = Some(pos);
                            }
                            skip_pointer = false;
                        }
                    }
                    if ended && !self.touches.values().any(|t| matches!(t.kind, Kind::Trackpad | Kind::Gesture)) {
                        self.gesture = false;
                    }
                    // Kept for pinch zoom and two-finger pan.
                    out.push(ev);
                }
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone if skip_pointer => {}
                other => {
                    skip_pointer = false;
                    out.push(other);
                }
            }
        }
        let m = self.modifiers();
        if m != Modifiers::NONE {
            raw.modifiers |= m;
            for ev in &mut out {
                match ev {
                    Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } | Event::MouseWheel { modifiers, .. } => *modifiers |= m,
                    _ => {}
                }
            }
        }
        raw.events = out;
        !self.pending.is_empty() || !self.held.is_empty()
    }

    fn trackpad_start(&mut self, tid: u64, pos: Pos2, now: f64, screen: Rect, out: &mut Vec<Event>) {
        self.trackpad = Some(tid);
        self.gesture = false;
        let cursor = *self.cursor.get_or_insert_with(|| screen.center());
        out.push(Event::PointerMoved(cursor));
        // Tap, then touch again and drag: drag with the primary button down.
        if let Some((t, at)) = self.last_tap
            && now - t < DOUBLE_TAP
            && (pos - at).length() < 3.0 * TAP_SLOP
            && !self.left_down
        {
            self.last_tap = None;
            self.left_down = true;
            self.tap_drag = true;
            out.push(pointer_button(cursor, PointerButton::Primary, true));
        }
    }

    fn move_cursor(&mut self, delta: Vec2, screen: Rect, out: &mut Vec<Event>) {
        let Some(c) = self.cursor else { return };
        let next = c + delta * self.config.sensitivity;
        let next = pos2(next.x.clamp(screen.left(), screen.right() - 1.0), next.y.clamp(screen.top(), screen.bottom() - 1.0));
        if next != c {
            self.cursor = Some(next);
            out.push(Event::PointerMoved(next));
        }
    }

    fn trackpad_end(&mut self, t: &Touch, phase: TouchPhase, now: f64, out: &mut Vec<Event>) {
        self.trackpad = None;
        let Some(cursor) = self.cursor else { return };
        if self.tap_drag {
            self.tap_drag = false;
            self.left_down = false;
            out.push(pointer_button(cursor, PointerButton::Primary, false));
        } else if phase == TouchPhase::End && !self.gesture && now - t.t0 < TAP_TIME && t.travel < TAP_SLOP {
            // A tap clicks: the release comes next frame, so it reads as press then release.
            out.push(pointer_button(cursor, PointerButton::Primary, true));
            self.pending.push(pointer_button(cursor, PointerButton::Primary, false));
            self.last_tap = Some((now, t.last));
        }
    }

    fn press(&mut self, i: usize, out: &mut Vec<Event>) {
        let Some(b) = self.config.buttons.get(i).cloned() else { return };
        if b.toggle && b.action.can_toggle() {
            // A latched toggle is released by the next tap (on release, below).
            if self.latched.contains(&i) {
                self.held.insert(i);
                return;
            }
        }
        self.held.insert(i);
        match &b.action {
            Action::Command { id } => self.queued.push(id.clone()),
            Action::Key { key } => {
                if let Some(k) = Key::from_name(key) {
                    out.push(key_event(k, true));
                }
            }
            Action::Modifier { .. } => {}
            Action::LeftClick => {
                if let Some(p) = self.target()
                    && !self.left_down
                {
                    self.left_down = true;
                    out.push(pointer_button(p, PointerButton::Primary, true));
                }
            }
            Action::RightClick => {
                if let Some(p) = self.target() {
                    out.push(pointer_button(p, PointerButton::Secondary, true));
                    self.pending.push(pointer_button(p, PointerButton::Secondary, false));
                }
            }
            Action::ToggleCursor => {
                self.config.cursor_mode = !self.config.cursor_mode;
                self.dirty = true;
                if self.config.cursor_mode {
                    if self.cursor.is_none() {
                        self.cursor = self.last_touch;
                    }
                    if let Some(c) = self.cursor {
                        out.push(Event::PointerMoved(c));
                    }
                } else {
                    out.push(Event::PointerGone);
                }
            }
            Action::Settings => {
                self.settings_open = !self.settings_open;
                if !self.settings_open {
                    self.editing = false;
                }
            }
        }
    }

    fn release(&mut self, i: usize, out: &mut Vec<Event>) {
        self.held.remove(&i);
        let Some(b) = self.config.buttons.get(i).cloned() else { return };
        if b.toggle && b.action.can_toggle() {
            if self.latched.insert(i) {
                // First tap: stays on.
                return;
            }
            self.latched.remove(&i);
        }
        match &b.action {
            Action::Key { key } => {
                if let Some(k) = Key::from_name(key) {
                    out.push(key_event(k, false));
                }
            }
            Action::LeftClick => {
                if self.left_down && !self.tap_drag {
                    self.left_down = false;
                    if let Some(p) = self.target() {
                        out.push(pointer_button(p, PointerButton::Primary, false));
                    }
                }
            }
            _ => {}
        }
    }

    /// Runs the commands buttons asked for. Call before the app's UI.
    pub fn run_queued(&mut self, app: &mut PhotocraftApp, ctx: &Context) {
        for id in std::mem::take(&mut self.queued) {
            if let Err(e) = crate::menus::invoke(app, ctx, &id, Value::Null) {
                log::info!("{id}: {e}");
            }
        }
    }

    /// Draws the buttons, the cursor and the settings window. Call after the app's UI.
    pub fn show(&mut self, ctx: &Context) {
        let screen = ctx.content_rect();
        let t = crate::theme::Tokens::get(ctx);
        let alpha = self.config.opacity;
        let mut moved: Option<(usize, Vec2)> = None;
        for (i, rect) in self.rects(screen) {
            let Some(b) = self.config.buttons.get(i) else { continue };
            let on = self.held.contains(&i)
                || self.latched.contains(&i)
                || (b.action == Action::ToggleCursor && self.config.cursor_mode)
                || (b.action == Action::Settings && self.settings_open);
            let editing = self.editing;
            egui::Area::new(Id::new(("pc-touch-button", i))).order(Order::Foreground).fixed_pos(rect.min).constrain(false).interactable(editing).show(
                ctx,
                |ui| {
                    let (r, resp) = ui.allocate_exact_size(rect.size(), if editing { Sense::drag() } else { Sense::hover() });
                    if editing && resp.dragged() {
                        moved = Some((i, resp.drag_delta()));
                    }
                    let fill = if on { t.accent.gamma_multiply(0.85 * alpha.max(0.5)) } else { Color32::from_black_alpha((200.0 * alpha) as u8) };
                    let stroke_c = if editing { t.accent } else { Color32::from_white_alpha((150.0 * alpha) as u8) };
                    ui.painter().circle(r.center(), r.width() / 2.0, fill, Stroke::new(if editing { 2.0 } else { 1.5 }, stroke_c));
                    let fg = Color32::from_white_alpha((255.0 * alpha.max(0.55)) as u8);
                    match b.icon.as_deref().filter(|n| crate::icons::exists(n)) {
                        Some(icon) => crate::icons::paint(ui, r, icon, (r.width() * 0.42).round(), fg),
                        None => {
                            let size = (r.width() * 0.3).clamp(10.0, 18.0);
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &b.label, egui::FontId::proportional(size), fg);
                        }
                    }
                },
            );
        }
        if let Some((i, d)) = moved
            && let Some(b) = self.config.buttons.get_mut(i)
            && screen.width() > 0.0
            && screen.height() > 0.0
        {
            b.x = (b.x + d.x / screen.width()).clamp(0.0, 1.0);
            b.y = (b.y + d.y / screen.height()).clamp(0.0, 1.0);
            self.dirty = true;
        }
        if self.cursor_active()
            && let Some(c) = self.cursor
        {
            paint_cursor(ctx, c, self.left_down);
        }
        if self.settings_open {
            self.settings_window(ctx, screen);
        }
    }

    fn settings_window(&mut self, ctx: &Context, screen: Rect) {
        let before = self.config.clone();
        let mut open = true;
        let presets = presets();
        egui::Window::new(l("Touch controls", "Controles táctiles"))
            .id(Id::new("pc-touch-settings"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(screen.width().min(340.0))
            .max_height(screen.height() * 0.85)
            .constrain_to(screen)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().interact_size.y = ui.spacing().interact_size.y.max(30.0);
                    let c = &mut self.config;
                    ui.checkbox(&mut c.visible, l("Show buttons", "Mostrar botones"));
                    ui.add(egui::Slider::new(&mut c.opacity, 0.1..=1.0).text(l("Opacity", "Opacidad")));
                    ui.separator();
                    ui.checkbox(&mut c.cursor_mode, l("Cursor mode (screen works as a trackpad)", "Modo cursor (la pantalla funciona como touchpad)"));
                    ui.add(egui::Slider::new(&mut c.sensitivity, 0.2..=5.0).step_by(0.1).text(l("Sensitivity", "Sensibilidad")));
                    ui.label(
                        egui::RichText::new(l(
                            "Tap: click · tap, then touch again and drag: drag · two fingers: zoom and pan.",
                            "Tocar: clic · tocar, volver a tocar y arrastrar: arrastrar · dos dedos: zoom y mover.",
                        ))
                        .small(),
                    );
                    ui.separator();
                    ui.checkbox(&mut self.editing, l("Move buttons (drag them)", "Mover botones (arrastralos)"));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(l("Buttons", "Botones")).strong());
                    let mut remove: Option<usize> = None;
                    let gears = self.config.buttons.iter().filter(|b| b.action == Action::Settings).count();
                    for (i, b) in self.config.buttons.iter_mut().enumerate() {
                        ui.push_id(("touch-btn", i), |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let current = presets.iter().position(|p| p.action == b.action);
                                let label = current.and_then(|n| presets.get(n)).map_or(b.label.as_str(), |p| p.name);
                                egui::ComboBox::from_id_salt("action").selected_text(label).width(170.0).show_ui(ui, |ui| {
                                    for p in &presets {
                                        if ui.selectable_label(Some(&p.action) == Some(&b.action), p.name).clicked() && p.action != b.action {
                                            b.action = p.action.clone();
                                            b.label = p.short.to_string();
                                            b.icon = p.icon.map(str::to_string);
                                            b.toggle = p.toggle;
                                        }
                                    }
                                });
                                let deletable = b.action != Action::Settings || gears > 1;
                                if ui.add_enabled(deletable, egui::Button::new(l("Remove", "Quitar"))).clicked() {
                                    remove = Some(i);
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.add(egui::Slider::new(&mut b.size, 28.0..=120.0).text(l("Size", "Tamaño")));
                                if b.action.can_toggle() {
                                    ui.checkbox(&mut b.toggle, l("Toggle", "Fijo"));
                                }
                            });
                        });
                        ui.add_space(2.0);
                    }
                    if let Some(i) = remove {
                        self.config.buttons.remove(i);
                        self.held.clear();
                        self.latched.clear();
                    }
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        let name = presets.get(self.add_choice).map_or("", |p| p.name);
                        egui::ComboBox::from_id_salt("add").selected_text(name).width(170.0).show_ui(ui, |ui| {
                            for (n, p) in presets.iter().enumerate() {
                                ui.selectable_value(&mut self.add_choice, n, p.name);
                            }
                        });
                        if ui.button(l("Add", "Agregar")).clicked()
                            && let Some(p) = presets.get(self.add_choice)
                        {
                            self.config.buttons.push(p.button(0.5, 0.5));
                            self.editing = true;
                        }
                    });
                    ui.add_space(4.0);
                    if ui.button(l("Restore defaults", "Restaurar por defecto")).clicked() {
                        self.config = Config::default();
                        self.held.clear();
                        self.latched.clear();
                    }
                });
            });
        if !open {
            self.settings_open = false;
            self.editing = false;
        }
        if self.config != before {
            self.dirty = true;
            if before.cursor_mode && !self.config.cursor_mode {
                self.pending.push(Event::PointerGone);
            }
        }
    }
}

fn pointer_button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE }
}

fn key_event(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

/// An arrow cursor with its tip at `at`.
fn paint_cursor(ctx: &Context, at: Pos2, pressed: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("pc-touch-cursor")));
    let s = 1.25;
    let pts: Vec<Pos2> =
        [(0.0, 0.0), (0.0, 17.0), (4.5, 13.0), (7.5, 20.0), (10.5, 18.8), (7.6, 12.0), (13.0, 12.0)].iter().map(|(x, y)| at + vec2(*x, *y) * s).collect();
    let fill = if pressed { Color32::from_rgb(120, 180, 255) } else { Color32::WHITE };
    painter.add(egui::Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
    painter.add(egui::Shape::closed_line(pts, Stroke::new(1.3, Color32::BLACK)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{RawInput, TouchDeviceId, TouchId};

    const SCREEN: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 400.0, y: 800.0 } };

    fn touch(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos, force: None }
    }

    /// What egui-winit sends for the touch it turns into the pointer.
    fn pointer_touch(id: u64, phase: TouchPhase, pos: Pos2) -> Vec<Event> {
        let mut v = vec![touch(id, phase, pos)];
        match phase {
            TouchPhase::Start => {
                v.push(Event::PointerMoved(pos));
                v.push(pointer_button(pos, PointerButton::Primary, true));
            }
            TouchPhase::Move => v.push(Event::PointerMoved(pos)),
            TouchPhase::End | TouchPhase::Cancel => {
                v.push(pointer_button(pos, PointerButton::Primary, false));
                v.push(Event::PointerGone);
            }
        }
        v
    }

    fn controls(buttons: Vec<Button>, cursor_mode: bool) -> TouchControls {
        TouchControls::new(Config { visible: true, opacity: 0.6, cursor_mode, sensitivity: 2.0, buttons })
    }

    fn button(action: Action, x: f32, y: f32, toggle: bool) -> Button {
        Button { action, label: "b".into(), icon: None, x, y, size: 50.0, toggle }
    }

    fn frame(tc: &mut TouchControls, now: f64, events: Vec<Event>) -> RawInput {
        let mut raw = RawInput { events, ..Default::default() };
        tc.process(SCREEN, now, &mut raw);
        raw
    }

    fn center(b: &Button) -> Pos2 {
        pos2(b.x * SCREEN.width(), b.y * SCREEN.height())
    }

    #[test]
    fn a_touch_on_a_command_button_runs_it_and_never_reaches_the_ui() {
        let undo = button(Action::Command { id: "edit.undo".into() }, 0.5, 0.5, false);
        let at = center(&undo);
        let mut tc = controls(vec![undo, button(Action::Settings, 0.1, 0.1, false)], false);
        let raw = frame(&mut tc, 0.0, pointer_touch(1, TouchPhase::Start, at));
        assert!(raw.events.is_empty(), "{:?}", raw.events);
        assert_eq!(tc.queued, vec!["edit.undo".to_string()]);
        let raw = frame(&mut tc, 0.1, pointer_touch(1, TouchPhase::End, at));
        assert!(raw.events.is_empty());
        // A touch elsewhere passes through untouched.
        let raw = frame(&mut tc, 0.2, pointer_touch(2, TouchPhase::Start, pos2(10.0, 700.0)));
        assert_eq!(raw.events.len(), 3);
    }

    #[test]
    fn a_held_modifier_applies_to_other_touches_and_keys() {
        let ctrl = button(Action::Modifier { modifier: Modifier::Ctrl }, 0.5, 0.5, false);
        let at = center(&ctrl);
        let mut tc = controls(vec![ctrl, button(Action::Settings, 0.1, 0.1, false)], false);
        frame(&mut tc, 0.0, pointer_touch(1, TouchPhase::Start, at));
        // A second finger is not egui-winit's pointer touch; a key typed meanwhile is.
        let raw = frame(&mut tc, 0.1, vec![touch(2, TouchPhase::Start, pos2(20.0, 700.0)), key_event(Key::Z, true)]);
        assert!(raw.modifiers.ctrl && raw.modifiers.command);
        assert!(raw.events.iter().any(|e| matches!(e, Event::Key { modifiers, .. } if modifiers.ctrl)));
        frame(&mut tc, 0.2, pointer_touch(1, TouchPhase::End, at));
        let raw = frame(&mut tc, 0.3, vec![key_event(Key::Z, true)]);
        assert!(!raw.modifiers.ctrl);
    }

    #[test]
    fn a_toggle_latches_until_the_next_tap() {
        let shift = button(Action::Modifier { modifier: Modifier::Shift }, 0.5, 0.5, true);
        let at = center(&shift);
        let mut tc = controls(vec![shift, button(Action::Settings, 0.1, 0.1, false)], false);
        frame(&mut tc, 0.0, pointer_touch(1, TouchPhase::Start, at));
        frame(&mut tc, 0.1, pointer_touch(1, TouchPhase::End, at));
        assert!(frame(&mut tc, 0.2, vec![]).modifiers.shift);
        frame(&mut tc, 0.3, pointer_touch(2, TouchPhase::Start, at));
        frame(&mut tc, 0.4, pointer_touch(2, TouchPhase::End, at));
        assert!(!frame(&mut tc, 0.5, vec![]).modifiers.shift);
    }

    #[test]
    fn cursor_mode_moves_the_cursor_by_the_sensitivity_and_a_tap_clicks() {
        let mut tc = controls(vec![button(Action::Settings, 0.05, 0.05, false)], true);
        let start = pos2(100.0, 400.0);
        let raw = frame(&mut tc, 0.0, pointer_touch(1, TouchPhase::Start, start));
        // The cursor starts in the middle; the finger's own pointer events are gone.
        assert!(raw.events.contains(&Event::PointerMoved(pos2(200.0, 400.0))));
        assert!(!raw.events.iter().any(|e| matches!(e, Event::PointerButton { .. })));
        let raw = frame(&mut tc, 0.05, pointer_touch(1, TouchPhase::Move, start + vec2(10.0, -5.0)));
        assert!(raw.events.contains(&Event::PointerMoved(pos2(220.0, 390.0))), "{:?}", raw.events);
        frame(&mut tc, 0.06, pointer_touch(1, TouchPhase::Move, start));
        // A quick tap with little movement: press now, release next frame, at the cursor.
        let raw = frame(&mut tc, 0.1, pointer_touch(1, TouchPhase::End, start));
        assert!(raw.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, pos, .. } if *pos == pos2(200.0, 400.0))));
        let raw = frame(&mut tc, 0.12, vec![]);
        assert!(raw.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: false, .. })));
        assert!(!raw.events.contains(&Event::PointerGone));
    }

    #[test]
    fn tap_then_touch_and_drag_drags_in_cursor_mode() {
        let mut tc = controls(vec![button(Action::Settings, 0.05, 0.05, false)], true);
        let p = pos2(100.0, 400.0);
        frame(&mut tc, 0.0, pointer_touch(1, TouchPhase::Start, p));
        frame(&mut tc, 0.1, pointer_touch(1, TouchPhase::End, p));
        frame(&mut tc, 0.12, vec![]);
        let raw = frame(&mut tc, 0.25, pointer_touch(2, TouchPhase::Start, p));
        assert!(raw.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. })));
        frame(&mut tc, 0.3, pointer_touch(2, TouchPhase::Move, p + vec2(30.0, 0.0)));
        let raw = frame(&mut tc, 0.5, pointer_touch(2, TouchPhase::End, p + vec2(30.0, 0.0)));
        assert!(raw.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: false, .. })));
    }

    #[test]
    fn a_second_finger_in_cursor_mode_is_a_gesture_not_a_click() {
        let mut tc = controls(vec![button(Action::Settings, 0.05, 0.05, false)], true);
        frame(&mut tc, 0.0, pointer_touch(1, TouchPhase::Start, pos2(100.0, 400.0)));
        let raw = frame(&mut tc, 0.02, vec![touch(2, TouchPhase::Start, pos2(200.0, 400.0))]);
        // The touch itself is kept, so egui still sees the pinch.
        assert_eq!(raw.events.len(), 1);
        frame(&mut tc, 0.05, vec![touch(2, TouchPhase::End, pos2(220.0, 400.0))]);
        let raw = frame(&mut tc, 0.1, pointer_touch(1, TouchPhase::End, pos2(100.0, 400.0)));
        assert!(!raw.events.iter().any(|e| matches!(e, Event::PointerButton { .. })));
    }

    #[test]
    fn the_config_round_trips_and_always_keeps_a_gear() {
        let c = Config::default();
        let json = serde_json::to_string(&c).unwrap_or_default();
        let back: Config = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(back, c);
        let broken = Config { opacity: f32::NAN, sensitivity: 99.0, buttons: vec![], ..Config::default() }.sanitized();
        assert_eq!(broken.opacity, 0.6);
        assert_eq!(broken.sensitivity, 5.0);
        assert!(broken.buttons.iter().any(|b| b.action == Action::Settings));
        // Every default button resolves to a key egui knows or a command id.
        for p in presets() {
            if let Action::Key { key } = &p.action {
                assert!(Key::from_name(key).is_some(), "{key}");
            }
        }
    }
}
