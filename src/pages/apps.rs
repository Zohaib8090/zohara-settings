use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;

pub fn build() -> gtk4::Widget {
    let prefs_page = adw::PreferencesPage::new();

    let apps_group = adw::PreferencesGroup::new();
    apps_group.set_title("Installed applications");

    let loading_row = adw::ActionRow::new();
    loading_row.set_title("Scanning installed apps…");
    let spinner = gtk4::Spinner::new();
    spinner.start();
    loading_row.add_suffix(&spinner);
    apps_group.add(&loading_row);

    // Search bar
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search apps…"));
    search.set_margin_start(12);
    search.set_margin_end(12);
    search.set_margin_top(8);
    search.set_margin_bottom(4);

    let apps_group_clone = apps_group.clone();
    let loading_row_clone = loading_row.clone();
    glib::spawn_future_local(async move {
        // Run single pacman query to get explicitly installed apps and size
        let result = tokio::process::Command::new("bash")
            .args(["-c", "pacman -Qe 2>/dev/null"])
            .output()
            .await;

        apps_group_clone.remove(&loading_row_clone);

        if let Ok(out) = result {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut count = 0;

            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.is_empty() { continue; }
                let pkg = parts[0];
                let ver = parts.get(1).copied().unwrap_or("");

                count += 1;
                let row = adw::ActionRow::new();
                row.set_title(pkg);
                row.set_subtitle(&format!("Native package • {}", ver));
                let icon = gtk4::Image::from_icon_name("application-x-executable-symbolic");
                row.add_prefix(&icon);
                apps_group_clone.add(&row);
            }

            if count == 0 {
                let empty_row = adw::ActionRow::new();
                empty_row.set_title("No user-installed applications found");
                apps_group_clone.add(&empty_row);
            }
        }
    });

    prefs_page.add(&apps_group);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());
    toolbar_view.set_content(Some(&prefs_page));
    toolbar_view.upcast()
}
