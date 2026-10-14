//! Compact layout for phones and small tablets.
//!
//! The desktop layout (title bar with every menu, options bar, toolbar, status bar and a right
//! dock at least 250 pt wide) needs about a thousand points of width. A phone in portrait has
//! 360–430, so the dock alone would take most of the screen and the menus would run off it.
//!
//! When the layout is compact (see [`active`]):
//! - the menus sit in one row that scrolls sideways with a swipe;
//! - the status bar and the right dock (with its icon rail) are hidden;
//! - a bottom bar holds Undo, Redo, Fit on Screen and one button per panel group;
//! - a panel group opens as a drawer over the canvas, from the right edge; tapping outside it or
//!   its close button closes it, and the canvas keeps its size underneath.
//!
//! The toolbar and the options bar stay as they are: the toolbar is one narrow column that
//! scrolls, and the options bar scrolls sideways.

use egui::{Color32, Context, Id, Margin, Order, Rect, Sense, Stroke, UiBuilder, pos2, vec2};
use serde_json::Value;

use crate::PhotocraftApp;
use crate::dock::Group;
use crate::icons;
use crate::theme::Tokens;

/// Whether the app may switch to the compact layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompactLayout {
    /// Always the desktop layout (desktop and web apps).
    #[default]
    Off,
    /// Compact whenever the window is narrower than [`MAX_WIDTH`] or lower than [`MAX_HEIGHT`]
    /// (a phone in either orientation, a tablet in portrait).
    Auto,
    /// Always compact.
    On,
}

/// Below this width (points) `Auto` goes compact.
pub const MAX_WIDTH: f32 = 900.0;
/// Below this height (points) `Auto` goes compact: a phone in landscape.
pub const MAX_HEIGHT: f32 = 520.0;

/// Top bar height (points).
pub const TOP_BAR_H: f32 = 40.0;
/// Bottom bar height (points); its buttons are big enough for a finger.
pub const BOTTOM_BAR_H: f32 = 52.0;
/// Side of a bottom-bar button (points).
const BUTTON: f32 = 42.0;

/// Whether a window of `size` (points) gets the compact layout under `mode`.
pub fn compact_for(mode: CompactLayout, size: egui::Vec2) -> bool {
    match mode {
        CompactLayout::Off => false,
        CompactLayout::On => true,
        CompactLayout::Auto => size.x < MAX_WIDTH || size.y < MAX_HEIGHT,
    }
}

/// Whether this frame uses the compact layout.
pub fn active(app: &PhotocraftApp, ctx: &Context) -> bool {
    compact_for(app.compact_layout, ctx.content_rect().size())
}

fn drawer_id() -> Id {
    Id::new("pc-mobile-drawer-group")
}

/// The panel group open in the drawer, if any.
pub fn open_group(ctx: &Context) -> Option<Group> {
    ctx.data(|d| d.get_temp::<Option<Group>>(drawer_id())).flatten()
}

/// Open `group` in the drawer, or close the drawer with `None`.
pub fn set_open_group(ctx: &Context, group: Option<Group>) {
    ctx.data_mut(|d| d.insert_temp(drawer_id(), group));
}

/// The top bar: every menu in one row that scrolls sideways.
pub fn top_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_top_bar").exact_size(TOP_BAR_H).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(Margin::symmetric(4, 0))).show(ui, |ui| {
        egui::ScrollArea::horizontal()
            .id_salt("pc-mobile-menus")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_height(TOP_BAR_H);
                ui.horizontal_centered(|ui| {
                    // Bigger titles than the desktop bar's: they are tapped, not clicked.
                    ui.style_mut().text_styles.entry(egui::TextStyle::Button).and_modify(|f| f.size = f.size.max(15.0));
                    crate::menus::menu_bar(app, ui);
                });
            });
    });
}

/// The panel groups the bottom bar opens: icon, label, group.
/// Studio floats Properties outside the dock; on a phone it opens in the drawer like the rest.
fn groups() -> [(&'static str, &'static str, Group); 5] {
    [
        ("layers", tl!("Layers"), Group::Layers),
        ("sliders-horizontal", tl!("Properties"), Group::Properties),
        ("palette", tl!("Color & Swatches"), Group::Color),
        ("clock", tl!("History"), Group::History),
        ("navigation", tl!("Navigator"), Group::Navigator),
    ]
}

fn run(app: &mut PhotocraftApp, ctx: &Context, id: &str) {
    if let Err(e) = crate::menus::invoke(app, ctx, id, Value::Null) {
        log::debug!("{id}: {e}");
    }
}

/// The bottom bar: Undo, Redo, Fit on Screen, then one button per panel group.
pub fn bottom_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let open = open_group(&ctx);
    egui::Panel::bottom("mobile_bottom_bar").exact_size(BOTTOM_BAR_H).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(Margin::symmetric(4, 0))).show(
        ui,
        |ui| {
            let r = ui.max_rect();
            ui.painter().line_segment([pos2(r.left() - 4.0, r.top()), pos2(r.right() + 4.0, r.top())], Stroke::new(1.0, t.separator));
            let groups = groups();
            let count = 3 + groups.len();
            // Spread the buttons over the whole width, with a little more room around the divider.
            let gap = ((ui.available_width() - BUTTON * count as f32) / (count as f32 - 1.0)).max(0.0);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let actions: [(&str, &str, &str); 3] =
                    [("undo-2", tl!("Undo"), "edit.undo"), ("redo-2", tl!("Redo"), "edit.redo"), ("maximize-2", tl!("Fit on Screen"), "view.fitOnScreen")];
                for (icon, tip, id) in actions {
                    let enabled = crate::menus::is_enabled(app, id);
                    let clicked = ui.add_enabled_ui(enabled, |ui| icons::button(ui, icon, BUTTON, false, tip)).inner.clicked();
                    if clicked {
                        run(app, &ctx, id);
                    }
                }
                for (icon, tip, g) in groups {
                    let on = open == Some(g);
                    if icons::button(ui, icon, BUTTON, on, tip).clicked() {
                        set_open_group(&ctx, if on { None } else { Some(g) });
                    }
                }
            });
        },
    );
}

/// The drawer's rect inside `screen` (the area between the top and bottom bars).
pub fn drawer_rect(screen: Rect) -> Rect {
    // Most of a phone's width, but never wider than a desktop dock.
    let w = (screen.width() * 0.88).clamp(screen.width().min(260.0), 380.0);
    Rect::from_min_max(pos2(screen.right() - w, screen.top()), screen.max)
}

/// The open panel group as a drawer over the canvas. Call after the panels and the canvas.
pub fn drawer(app: &mut PhotocraftApp, ctx: &Context) {
    let Some(group) = open_group(ctx) else { return };
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let area = Rect::from_min_max(pos2(screen.left(), screen.top() + TOP_BAR_H), pos2(screen.right(), screen.bottom() - BOTTOM_BAR_H));
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return;
    }
    let rect = drawer_rect(area);

    // Dimmed backdrop over the canvas: a tap on it closes the drawer.
    let backdrop = egui::Area::new(Id::new("pc-mobile-drawer-backdrop")).order(Order::Middle).fixed_pos(area.min).constrain(false).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(area.size(), Sense::click());
        ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(90));
        resp.clicked()
    });
    if backdrop.inner {
        set_open_group(ctx, None);
        return;
    }

    let mut close = false;
    egui::Area::new(Id::new("pc-mobile-drawer")).order(Order::Middle).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        let (frame_rect, _) = ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
        ui.painter().rect_filled(frame_rect, 0.0, t.dock);
        let x = if crate::direction::is_rtl() { frame_rect.right() } else { frame_rect.left() };
        ui.painter().line_segment([pos2(x, frame_rect.top()), pos2(x, frame_rect.bottom())], Stroke::new(1.0, t.separator));
        let inner = frame_rect.shrink(6.0);

        // Header: the group's name and a close button.
        let header = Rect::from_min_size(inner.min, vec2(inner.width(), BUTTON - 6.0));
        let mut head = ui.new_child(UiBuilder::new().max_rect(header).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let title = groups().into_iter().find(|(_, _, g)| *g == group).map_or("", |(_, name, _)| name);
        head.label(egui::RichText::new(title).strong().size(15.0).color(t.text));
        head.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, "x", BUTTON - 8.0, false, tl!("Close")).clicked() {
                close = true;
            }
        });

        // Body: the group's tabs, as in the dock.
        let body = Rect::from_min_max(pos2(inner.left(), header.bottom() + 4.0), inner.max);
        if body.height() <= 0.0 {
            return;
        }
        let mut child = ui.new_child(UiBuilder::new().id_salt(("mobile-drawer", group.key())).max_rect(body));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        group_card(app, &mut child, group);
    });
    if close {
        set_open_group(ctx, None);
    }
}

/// One panel group with its tabs, filling `ui`; the tab shown is the dock's, so both agree.
fn group_card(app: &mut PhotocraftApp, ui: &mut egui::Ui, g: Group) {
    let t = Tokens::get(ui.ctx());
    let visible = app.ui.dock.visible_tabs(g, t.pro);
    if visible.is_empty() {
        return;
    }
    let indices: Vec<usize> = visible.iter().map(|(i, _)| *i).collect();
    let tabs: Vec<&str> = visible.iter().map(|(_, name)| *name).collect();
    let before = *g.tab_mut(&mut app.ui.dock_tabs);
    let mut sel = indices.iter().position(|i| *i == before).unwrap_or(0);
    ui.spacing_mut().item_spacing.y = if t.pro { 0.0 } else { 6.0 };
    let _ = crate::widgets::card_ex(ui, &format!("mobile-{}", g.key()), &tabs, &mut sel, false, false, |ui, shown_tab| {
        let Some(&tab) = indices.get(shown_tab) else { return };
        let inner = ui.available_height().max(0.0);
        if g.scrolls_itself(tab) {
            ui.set_min_height(inner);
            crate::panels::dock_body(app, ui, g, tab);
        } else {
            ui.spacing_mut().scroll.fade.strength = 0.0;
            egui::ScrollArea::vertical()
                .id_salt(("mobile-scroll", g.key(), tab))
                .max_height(inner)
                .auto_shrink([false, false])
                .show(ui, |ui| crate::panels::dock_body(app, ui, g, tab));
        }
    });
    if let Some(&tab) = indices.get(sel) {
        *g.tab_mut(&mut app.ui.dock_tabs) = tab;
    }
}

/// Two fingers on the canvas pan the view by this much this frame (points, content direction);
/// the pinch itself zooms through `zoom_delta` (`wheel_nav`).
pub fn two_finger_pan(ctx: &Context, over: Rect) -> Option<egui::Vec2> {
    let mt = ctx.multi_touch()?;
    (mt.num_touches >= 2 && over.contains(mt.center_pos) && mt.translation_delta != egui::Vec2::ZERO).then_some(mt.translation_delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_compact_on_phones_only() {
        // Phone portrait and landscape, tablet portrait: compact.
        assert!(compact_for(CompactLayout::Auto, vec2(392.0, 850.0)));
        assert!(compact_for(CompactLayout::Auto, vec2(850.0, 392.0)));
        assert!(compact_for(CompactLayout::Auto, vec2(800.0, 1280.0)));
        // Tablet landscape and desktop windows: the desktop layout.
        assert!(!compact_for(CompactLayout::Auto, vec2(1280.0, 800.0)));
        assert!(!compact_for(CompactLayout::Auto, vec2(1920.0, 1080.0)));
        assert!(!compact_for(CompactLayout::Off, vec2(392.0, 850.0)));
        assert!(compact_for(CompactLayout::On, vec2(1920.0, 1080.0)));
    }

    #[test]
    fn the_drawer_fits_the_screen() {
        let phone = Rect::from_min_size(pos2(0.0, 40.0), vec2(392.0, 700.0));
        let r = drawer_rect(phone);
        assert!(r.width() <= phone.width() && r.width() >= 260.0);
        assert_eq!(r.right(), phone.right());
        let tiny = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 300.0));
        assert!(drawer_rect(tiny).width() <= 200.0);
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(880.0, 300.0));
        assert_eq!(drawer_rect(wide).width(), 380.0);
    }
}
