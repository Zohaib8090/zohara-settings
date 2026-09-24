//! Touchpad settings -- per-device enable, tap to click, disable while typing,
//! speed, acceleration, natural scrolling (via KWin).

use gtk4::prelude::*;

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);

    root.append(
        &gtk4::Label::builder()
            .label("Touchpad")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let devices = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    super::input_devices::populate(&devices, true);
    root.append(&devices);

    scroll.set_child(Some(&root));
    scroll.upcast()
}
