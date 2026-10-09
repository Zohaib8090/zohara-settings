//! Keeping the numbers on a page current. A page asks for a tick every second or so; the tick runs fastest while the page
//! is on screen in the window you are using, slower when the window is visible but not in front, and not at all for a
//! page that is not shown. The work itself
//! is done by the caller (normally in a background thread, with the result applied back on the UI thread).

use gtk4::glib;
use gtk4::prelude::*;
use std::time::Duration;

/// How much slower a page refreshes while it is visible but another window has the focus (looked at, not used).
const UNFOCUSED_FACTOR: u32 = 5;

/// Calls `tick` every `secs` seconds while `widget` is on screen in the window you are using, and every
/// `secs * UNFOCUSED_FACTOR` seconds while it is on screen but another window is in front. Nothing runs for a page
/// that is not selected. When a page comes back into view it refreshes at once, so it never shows old numbers.
/// Stops for good when the widget is destroyed.
pub fn every(widget: &impl IsA<gtk4::Widget>, secs: u32, tick: impl Fn() + 'static) {
    const STEP_MS: u32 = 250;
    let weak = widget.as_ref().downgrade();
    let tick = std::rc::Rc::new(tick);
    let was_showing = std::cell::Cell::new(false);
    let mut elapsed_ms: u32 = 0;
    glib::timeout_add_local(Duration::from_millis(STEP_MS as u64), move || {
        let Some(w) = weak.upgrade() else { return glib::ControlFlow::Break };
        let Some(focused) = showing(&w) else {
            was_showing.set(false);
            elapsed_ms = 0;
            return glib::ControlFlow::Continue;
        };
        elapsed_ms += STEP_MS;
        let period_ms = secs * 1000 * if focused { 1 } else { UNFOCUSED_FACTOR };
        // coming back into view: refresh now instead of waiting a full period
        if !was_showing.replace(true) || elapsed_ms >= period_ms {
            elapsed_ms = 0;
            tick();
        }
        glib::ControlFlow::Continue
    });
}

/// `None` when the widget is not on screen; otherwise whether its window has the focus.
fn showing(w: &gtk4::Widget) -> Option<bool> {
    if !w.is_mapped() {
        return None;
    }
    let win = w.root()?.downcast::<gtk4::Window>().ok()?;
    Some(win.is_active())
}
