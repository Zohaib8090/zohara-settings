use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;

pub fn build() -> gtk4::Widget {
    let prefs_page = adw::PreferencesPage::new();

    // ── Users ─────────────────────────────────────────────────────────────────
    let users_group = adw::PreferencesGroup::new();
    users_group.set_title("Local accounts");

    let add_btn = gtk4::Button::builder()
        .label("Add account")
        .css_classes(vec!["flat".to_string()])
        .icon_name("list-add-symbolic")
        .build();
    users_group.set_header_suffix(Some(&add_btn));

    let users_group_clone = users_group.clone();
    glib::spawn_future_local(async move {
        if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
            let groups = std::fs::read_to_string("/etc/group").unwrap_or_default();
            let mut count = 0;

            for line in passwd.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() < 7 { continue; }
                let username = parts[0];
                let uid: u32 = parts[2].parse().unwrap_or(0);
                if uid < 1000 || uid >= 65534 { continue; }

                count += 1;
                let gecos = parts[4].split(',').next().unwrap_or("").trim();
                let is_admin = groups.lines().any(|gl| {
                    (gl.starts_with("wheel:") || gl.starts_with("sudo:")) && gl.contains(username)
                });

                let row = adw::ActionRow::new();
                let display_name = if gecos.is_empty() || gecos == username {
                    username.to_string()
                } else {
                    format!("{} ({})", gecos, username)
                };
                row.set_title(&display_name);
                row.set_subtitle(&format!(
                    "{} • UID {}",
                    if is_admin { "Administrator" } else { "Standard user" },
                    uid
                ));

                let icon = gtk4::Image::from_icon_name(
                    if is_admin { "avatar-default-symbolic" } else { "system-users-symbolic" }
                );
                row.add_prefix(&icon);

                users_group_clone.add(&row);
            }

            if count == 0 {
                let empty_row = adw::ActionRow::new();
                empty_row.set_title("No local user accounts found");
                users_group_clone.add(&empty_row);
            }
        }
    });

    add_btn.connect_clicked(|_| {
        let _ = std::process::Command::new("kcmshell6").arg("kcm_users").spawn();
    });

    prefs_page.add(&users_group);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());
    toolbar_view.set_content(Some(&prefs_page));
    toolbar_view.upcast()
}
