//! The compact workspace (StuCent #10966): a host (the StuCent page) asks for it with the bridge's
//! `compact {on}` message when the editor sits in a tablet-width frame (768-1024 px) and is
//! driven by touch. Compact does four things:
//!
//! - the canvas keeps at least [`MIN_CANVAS`] points of width: the layers/properties dock is a
//!   drawer, closed until the 44 pt icon rail asks for it;
//! - the tool rail is 48 pt wide with 44 pt buttons;
//! - the title bar is 44 pt tall;
//! - menu titles and menu rows are at least [`TOUCH`] (44 pt) tall.
//!
//! The flag is process-wide (one editor per page); the rules are pure functions so they are tested
//! natively.

use std::sync::atomic::{AtomicBool, Ordering};

/// The least size of a touch target, in points (Apple HIG, Material).
pub const TOUCH: f32 = 44.0;
/// The tool rail's width in compact mode.
pub const TOOL_RAIL: f32 = 48.0;
/// The canvas keeps at least this width (and height) when the drawer is closed.
pub const MIN_CANVAS: f32 = 600.0;
/// The drawer's width (the dock's own range is 250-520).
pub const DRAWER: f32 = 280.0;

static COMPACT: AtomicBool = AtomicBool::new(false);
static DRAWER_OPEN: AtomicBool = AtomicBool::new(false);

/// Is the compact workspace on?
pub fn on() -> bool {
    COMPACT.load(Ordering::Relaxed)
}

/// Turn the compact workspace on or off; the drawer starts closed either way.
pub fn set_on(on: bool) {
    COMPACT.store(on, Ordering::Relaxed);
    DRAWER_OPEN.store(false, Ordering::Relaxed);
}

/// Is the dock drawer open? Only meaningful while [`on`].
pub fn drawer_open() -> bool {
    DRAWER_OPEN.load(Ordering::Relaxed)
}

pub fn set_drawer(open: bool) {
    DRAWER_OPEN.store(open, Ordering::Relaxed);
}

/// Does the right dock take part of the screen? Always when not compact; as a drawer, only while open.
pub fn dock_visible(compact: bool, drawer_open: bool) -> bool {
    !compact || drawer_open
}

/// What a rail button does in compact mode: `(open the drawer, reveal the group)`. A button for a
/// group that is showing in an open drawer closes the drawer; any other opens it on that group.
pub fn rail_action(drawer_open: bool, group_shown: bool) -> RailAction {
    if drawer_open && group_shown { RailAction::CloseDrawer } else { RailAction::OpenOn }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailAction {
    CloseDrawer,
    OpenOn,
}

/// The width the canvas has left on a screen `screen_w` wide, with the tool rail, the icon rail
/// (`icon_rail`) and, if open, the drawer.
pub fn canvas_width(screen_w: f32, icon_rail: f32, drawer_open: bool) -> f32 {
    screen_w - TOOL_RAIL - icon_rail - if drawer_open { DRAWER } else { 0.0 }
}

/// Touch-sized rows and buttons for the ui (menus, menu bar).
pub fn touch_spacing(ui: &mut egui::Ui) {
    let s = ui.spacing_mut();
    s.interact_size.y = s.interact_size.y.max(TOUCH);
    s.button_padding.y = s.button_padding.y.max(10.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawer_is_the_only_way_the_dock_shows_in_compact() {
        assert!(dock_visible(false, false));
        assert!(!dock_visible(true, false));
        assert!(dock_visible(true, true));
    }

    #[test]
    fn rail_button_toggles_the_drawer_on_its_group() {
        assert_eq!(rail_action(false, false), RailAction::OpenOn);
        assert_eq!(rail_action(false, true), RailAction::OpenOn);
        assert_eq!(rail_action(true, false), RailAction::OpenOn);
        assert_eq!(rail_action(true, true), RailAction::CloseDrawer);
    }

    #[test]
    fn a_768_wide_tablet_keeps_a_600_canvas_with_the_drawer_closed() {
        for w in [768.0, 820.0, 1024.0] {
            assert!(canvas_width(w, 44.0, false) >= MIN_CANVAS, "{w}");
        }
        // The drawer takes room: open, it covers the canvas on the narrowest tablets, by design.
        assert!(canvas_width(768.0, 44.0, true) < MIN_CANVAS);
    }

    #[test]
    fn touch_spacing_raises_rows_to_44() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            touch_spacing(ui);
            assert!(ui.spacing().interact_size.y >= TOUCH);
        })
        .textures_delta
        .clear();
    }

    #[test]
    fn set_on_closes_the_drawer() {
        set_on(true);
        set_drawer(true);
        assert!(on() && drawer_open());
        set_on(false);
        assert!(!on() && !drawer_open());
    }
}
