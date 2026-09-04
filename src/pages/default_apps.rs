//! Default apps — pick the default browser, terminal, file manager, image viewer.
//!
//! Uses xdg-mime to set the default app for each MIME type. Persists
//! across reboots because xdg-mime writes to ~/.config/mimeapps.list.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

fn current_default(mime: &str) -> String {
    Command::new("xdg-mime")
        .args(["query", "default", mime])
        .output()
        .ok()
        .and_then(|o| if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else { None })
        .unwrap_or_else(|| "(none)".to_string())
}

fn set_default(mime: &str, desktop: &str) {
    let _ = Command::new("xdg-mime")
        .args(["default", desktop, mime])
        .status();
}

fn pick_app() -> Option<String> {
    // Open a file picker that filters to .desktop files.
    None
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);

    let title = gtk4::Label::builder()
        .label("Default apps")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    let types: &[(&str, &str, &str, &str)] = &[
        ("Web browser", "text/html", "applications-internet", "firefox.desktop"),
        ("Email",       "x-scheme-handler/mailto", "mail-unread-symbolic", ""),
        ("Terminal",    "application/x-terminal", "utilities-terminal", ""),
        ("File manager","inode/directory", "system-file-manager", ""),
        ("Image viewer","image/jpeg", "image-x-generic", ""),
        ("Video player","video/mp4", "video-x-generic", ""),
        ("Music player", "audio/mpeg", "audio-x-generic", ""),
        ("Text editor", "text/plain", "text-x-generic", ""),
    ];

    for (label, mime, icon, fallback) in types {
        let row = adw::ActionRow::new();
        row.set_title(label);
        row.set_subtitle(&current_default(mime));
        row.add_prefix(&gtk4::Image::from_icon_name(icon));
        let btn = gtk4::Button::builder()
            .label("Choose")
            .valign(gtk4::Align::Center)
            .css_classes(vec!["win11-secondary-btn".to_string()])
            .build();
        let mime_owned = mime.to_string();
        let label_owned = label.to_string();
        btn.connect_clicked(move |btn| {
            // Open a file dialog filtered to .desktop files
            let parent = btn.root().and_downcast::<gtk4::Window>();
            let fd = gtk4::FileDialog::new();
            fd.set_title(&format!("Pick default for {label_owned}"));
            let mime_clone = mime_owned.clone();
            fd.open(parent.as_ref(), None::<&gtk4::gio::Cancellable>, move |res| {
                if let Ok(file) = res {
                    if let Some(p) = file.path() {
                        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                        if !name.is_empty() {
                            set_default(&mime_clone, &format!("{name}.desktop"));
                        }
                    }
                }
            });
        });
        row.add_suffix(&btn);
        let _ = fallback;
        row.set_activatable(false);
        rows.append(&row);
    }

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
