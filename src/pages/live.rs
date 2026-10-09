//! Keeping the numbers on a page current. A page asks for a tick every few seconds; the tick only runs while the page
//! is on screen and the Settings window is the one in front, so a closed or hidden page costs nothing. The work itself
//! is done by the caller (normally in a background thread, with the result applied back on the UI thread).

use gtk4::glib;
use gtk4::prelude::*;
use std::time::Duration;

/// Calls `tick` every `secs` seconds while `widget` is visible and its window is active, once at once when the page
/// comes back into view (so it never shows old numbers). Stops for good when the widget is destroyed.
pub fn every(widget: &impl IsA<gtk4::Widget>, secs: u32, tick: impl Fn() + 'static) {
    let weak = widget.as_ref().downgrade();
    let tick = std::rc::Rc::new(tick);
    let was_showing = std::cell::Cell::new(false);
    glib::timeout_add_local(Duration::from_millis(500), {
        let (weak, tick) = (weak.clone(), tick.clone());
        let mut elapsed_ms: u32 = 0;
        move || {
            let Some(w) = weak.upgrade() else { return glib::ControlFlow::Break };
            let showing = is_showing(&w);
            if !showing {
                was_showing.set(false);
                elapsed_ms = 0;
                return glib::ControlFlow::Continue;
            }
            elapsed_ms += 500;
            // coming back into view: refresh now instead of waiting a full period
            if !was_showing.replace(true) || elapsed_ms >= secs * 1000 {
                elapsed_ms = 0;
                tick();
            }
            glib::ControlFlow::Continue
        }
    });
}

/// On screen (mapped, so not on a page that is not selected) in a window that has the focus.
fn is_showing(w: &gtk4::Widget) -> bool {
    w.is_mapped() && w.root().and_then(|r| r.downcast::<gtk4::Window>().ok()).map(|win| win.is_active()).unwrap_or(false)
}
