//! "Find a setting": search over the individual options, not just the pages, like Windows 11's Settings search.
//!
//! Typing "wifi" lists "Wi-Fi" under "Network & internet", "Hotspot", and so on. Choosing a result opens that page and
//! scrolls to the row, which flashes once so the eye finds it. The index (`ENTRIES`) is written by hand from what each
//! page really shows; `tests::every_entry_is_on_its_page` fails when a row is renamed and the index was not updated.

use gtk4::prelude::*;
use libadwaita::prelude::*;

/// One thing that can be found.
pub struct Entry {
    /// What the row says on the page (also what is matched in the page to scroll to it).
    pub title: &'static str,
    /// The page's label in `PAGES` (hidden pages like "Display" count too).
    pub page: &'static str,
    /// Other words that should find it: synonyms, the old names, things people type.
    pub keywords: &'static str,
}

/// Lowercase and keep only letters and digits, so "Wi-Fi", "wi fi" and "wifi" are the same word.
pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// How well `entry` matches the query: None = not at all. Every word of the query must match somewhere.
fn score(entry: &Entry, page_label: &str, words: &[String]) -> Option<u32> {
    let title = normalize(entry.title);
    let page = normalize(page_label);
    let keys = normalize(entry.keywords);
    let mut total = 0;
    for w in words {
        let s = if title == *w {
            100
        } else if title.starts_with(w.as_str()) {
            60
        } else if title.contains(w.as_str()) {
            40
        } else if entry.keywords.split_whitespace().any(|k| normalize(k).starts_with(w.as_str())) {
            25
        } else if keys.contains(w.as_str()) {
            15
        } else if page.contains(w.as_str()) {
            8
        } else {
            return None;
        };
        total += s;
    }
    Some(total)
}

/// Entries matching the query, best first, at most `limit`.
pub fn find(entries: &'static [Entry], query: &str, limit: usize) -> Vec<&'static Entry> {
    let words: Vec<String> = query.split_whitespace().map(normalize).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u32, usize, &'static Entry)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| score(e, e.page, &words).map(|s| (s, i, e)))
        .collect();
    // Best score first; ties keep the order of the list (which is the order people expect on a page).
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().take(limit).map(|(_, _, e)| e).collect()
}

/// Every widget in the tree with a title or text a person reads: (text, widget).
pub fn titles_in(root: &gtk4::Widget) -> Vec<(String, gtk4::Widget)> {
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(w) = stack.pop() {
        if let Some(row) = w.downcast_ref::<libadwaita::PreferencesRow>() {
            let t = row.title();
            if !t.is_empty() {
                out.push((t.to_string(), w.clone()));
            }
        } else if let Some(g) = w.downcast_ref::<libadwaita::PreferencesGroup>() {
            let t = g.title();
            if !t.is_empty() {
                out.push((t.to_string(), w.clone()));
            }
        } else if let Some(l) = w.downcast_ref::<gtk4::Label>() {
            let t = l.text();
            if !t.is_empty() {
                out.push((t.to_string(), w.clone()));
            }
        } else if let Some(b) = w.downcast_ref::<gtk4::Button>() {
            if let Some(t) = b.label() {
                if !t.is_empty() {
                    out.push((t.to_string(), w.clone()));
                }
            }
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            c = child.next_sibling();
            stack.push(child);
        }
    }
    out
}

macro_rules! e {
    ($title:expr, $page:expr, $kw:expr) => {
        Entry { title: $title, page: $page, keywords: $kw }
    };
}

/// Every page, then the rows people look for. Titles are exactly what the page shows.
pub static ENTRIES: &[Entry] = &[
    // The pages themselves (sidebar and hidden ones).
    e!("Home", "Home", "overview start"),
    e!("System", "System", "device hardware memory processor utilities"),
    e!("Bluetooth & devices", "Bluetooth & devices", "bluetooth pairing mouse touchpad keyboard printer camera webcam phone"),
    e!("Network & internet", "Network & internet", "wifi wi-fi wireless ethernet cable vpn proxy hotspot airplane internet"),
    e!("Personalization", "Personalization", "theme colours background wallpaper accent dark light mode lock screen fonts taskbar start"),
    e!("Apps", "Apps", "installed apps startup web apps uninstall programs"),
    e!("Accounts", "Accounts", "user users account password login sign-in"),
    e!("Time & language", "Time & language", "date time clock timezone language region"),
    e!("Gaming", "Gaming", "game games controller steam"),
    e!("Accessibility", "Accessibility", "ease of access vision hearing screen reader"),
    e!("Privacy & security", "Privacy & security", "firewall camera microphone location permissions history"),
    e!("Zohara Update", "Zohara Update", "update updates upgrade check for updates os system windows update restore snapshot channel history"),
    e!("Display", "Display", "monitor screen resolution brightness refresh rate night light scale"),
    e!("Sound", "Sound", "volume audio speakers microphone output input"),
    e!("Notifications", "Notifications", "do not disturb alerts popups"),
    e!("Power & battery", "Power & battery", "battery sleep suspend lid power mode energy saver"),
    e!("Storage", "Storage", "disk drive space cleanup usb"),
    e!("Mouse", "Mouse", "pointer cursor speed buttons scrolling click"),
    e!("Touchpad", "Touchpad", "trackpad taps scrolling gestures"),
    e!("Keyboard", "Keyboard", "typing layout shortcuts keys"),
    e!("Printers", "Printers", "printing print scanner cups"),
    e!("Default apps", "Default apps", "default browser email file types open with"),
    e!("About", "About", "device info version kernel processor memory ram"),
    e!("Zohara Link", "Zohara Link", "phone android connect"),
    e!("Troubleshoot", "Troubleshoot", "fix problem repair logs report health"),
    // Network & internet
    e!("Wi-Fi", "Network & internet", "wifi wireless network connect internet known networks metered password"),
    e!("Available networks", "Network & internet", "wifi scan connect wireless"),
    e!("Airplane mode", "Network & internet", "flight wireless radio off"),
    e!("Mobile hotspot", "Network & internet", "hotspot tethering share internet"),
    e!("VPN", "Network & internet", "vpn wireguard openvpn tunnel private"),
    e!("Proxy", "Network & internet", "proxy http https socks"),
    e!("Speed test", "Network & internet", "internet speed download upload ping bandwidth"),
    e!("Network adapters", "Network & internet", "ethernet cable lan adapter ip address"),
    e!("Wi-Fi adapter", "Network & internet", "no wifi missing driver adapter not detected not working wireless card broken"),
    // System
    e!("Rename device", "System", "computer name hostname"),
    e!("User manager", "System", "users accounts add remove"),
    e!("Migration", "System", "import linux migrate move files"),
    e!("Memory", "System", "ram"),
    e!("Processor", "System", "cpu"),
    // Bluetooth & devices
    e!("Bluetooth", "Bluetooth & devices", "pair pairing wireless devices"),
    e!("Add a device", "Bluetooth & devices", "pair bluetooth pairing search"),
    e!("Your devices", "Bluetooth & devices", "paired bluetooth"),
    e!("Phone", "Bluetooth & devices", "android link zohara link"),
    // Display
    e!("Resolution & refresh rate", "Display", "resolution refresh rate hz monitor screen size"),
    e!("Scale", "Display", "scaling dpi zoom size text bigger"),
    e!("Orientation", "Display", "rotate landscape portrait"),
    e!("Night light", "Display", "night color blue light warm eye strain"),
    // Sound
    e!("Output device", "Sound", "speakers headphones audio volume"),
    e!("Input device", "Sound", "microphone mic"),
    e!("Volume mixer", "Sound", "app volume per app"),
    e!("Test speakers", "Sound", "sound test left right"),
    // Notifications
    e!("Do not disturb", "Notifications", "dnd quiet focus silence"),
    e!("Pop-up duration", "Notifications", "popup seconds notification disappears"),
    e!("Show pop-ups", "Notifications", "popups banners"),
    e!("Show in notification history", "Notifications", "history"),
    e!("Show badges on the app icon", "Notifications", "badge count"),
    // Power & battery
    e!("Power mode", "Power & battery", "performance balanced power saver battery"),
    e!("Dim screen after", "Power & battery", "brightness idle"),
    e!("Turn off screen after", "Power & battery", "screen timeout monitor off display"),
    e!("Sleep after", "Power & battery", "suspend idle"),
    e!("When the lid is closed", "Power & battery", "laptop lid close suspend"),
    e!("When the power button is pressed", "Power & battery", "shutdown button"),
    // Storage
    e!("Storage Sense", "Storage", "free up space cleanup automatic"),
    e!("Clean up automatically", "Storage", "storage sense old files"),
    e!("System restore points", "Storage", "snapshots restore rollback"),
    e!("Trash", "Storage", "recycle bin empty deleted"),
    e!("Drives", "Storage", "disk usb eject removable"),
    e!("App caches", "Storage", "cache temporary"),
    e!("Old package versions", "Storage", "pacman cache packages"),
    e!("What's using space", "Storage", "disk usage large files"),
    // Personalization
    e!("Themes", "Personalization", "theme desktop style"),
    e!("Colors", "Personalization", "accent colour color dark mode light mode"),
    e!("Choose your mode", "Personalization", "dark light theme mode"),
    e!("Background", "Personalization", "wallpaper picture desktop image photo"),
    e!("Lock screen", "Personalization", "screen lock timeout inactivity"),
    e!("Fonts", "Personalization", "font family size text"),
    e!("Taskbar", "Personalization", "panel alignment center left"),
    e!("Taskbar alignment", "Personalization", "centre center left icons panel"),
    e!("Taskbar position", "Personalization", "taskbar top bottom left right edge screen panel"),
    e!("System font", "Personalization", "font family typeface text"),
    e!("Font size", "Personalization", "text size bigger smaller"),
    e!("Start", "Personalization", "menu launcher"),
    e!("Text input", "Personalization", "typing suggestions emoji"),
    e!("Dynamic Lighting", "Personalization", "rgb leds keyboard lighting"),
    e!("Transparency effects", "Personalization", "translucent blur glass"),
    // Privacy & security
    e!("Firewall", "Privacy & security", "ufw ports network security block incoming"),
    e!("Firewall rules", "Privacy & security", "ufw allow deny ports"),
    e!("Camera", "Privacy & security", "webcam permission"),
    e!("Microphone", "Privacy & security", "mic permission"),
    e!("Location", "Privacy & security", "gps services permission"),
    e!("Recent activity", "Privacy & security", "history apps used"),
    e!("Access indicators", "Privacy & security", "indicator dot microphone camera tray"),
    e!("Device access", "Privacy & security", "camera microphone location"),
    // Zohara Update
    e!("Update history", "Zohara Update", "installed packages logs upgrades"),
    e!("Pause updates", "Zohara Update", "delay stop automatic"),
    e!("Advanced options", "Zohara Update", "channel cache cleanup"),
    e!("Update channel", "Zohara Update", "stable beta alpha channel preview insider newest builds"),
    e!("Undo the last update", "Zohara Update", "go back rollback revert downgrade"),
    e!("Restore the whole system", "Zohara Update", "restore points snapshots rollback btrfs"),
    // Apps
    e!("Installed apps", "Apps", "uninstall remove programs"),
    e!("Startup", "Apps", "autostart login apps boot"),
    e!("App sources", "Apps", "flathub repositories flatpak"),
    e!("Flathub", "Apps", "flatpak store apps"),
    e!("Offline maps", "Apps", "maps download"),
    e!("Apps for websites", "Apps", "web apps pwa"),
    e!("Video playback", "Apps", "hardware decoding codec gpu"),
    // Gaming
    e!("Game stores", "Gaming", "steam lutris heroic bottles"),
    e!("Steam", "Gaming", "games store valve"),
    e!("Lutris", "Gaming", "games wine"),
    e!("Bottles", "Gaming", "windows games apps wine"),
    e!("Heroic Games Launcher", "Gaming", "epic gog amazon games"),
    e!("Game Mode", "Gaming", "gamemode performance"),
    e!("Performance overlay", "Gaming", "mangohud fps frames"),
    // Time & language
    e!("Language & region", "Time & language", "locale country"),
    e!("Regional format", "Time & language", "date number format"),
    e!("System language", "Time & language", "display language locale"),
    e!("Date & time", "Time & language", "clock"),
    e!("Time zone", "Time & language", "timezone clock location"),
    e!("Set time automatically", "Time & language", "clock ntp network time"),
    // Accessibility
    e!("High contrast", "Accessibility", "vision colours"),
    e!("Cursor size", "Accessibility", "mouse pointer big large"),
    e!("Text size", "Accessibility", "font zoom large bigger"),
    e!("Reduce animations", "Accessibility", "motion effects"),
    e!("Visual bell", "Accessibility", "hearing flash alert"),
    e!("Screen reader", "Accessibility", "orca blind speech"),
    e!("Speaking rate", "Accessibility", "speech speed voice"),
    e!("Voice typing", "Accessibility", "dictation speech to text"),
    e!("Test voice", "Accessibility", "speech sample"),
    // Gaming
    e!("Your games", "Gaming", "steam lutris heroic epic gog library installed games play"),
    e!("My games", "Gaming", "add game manually custom non-steam program exe appimage"),
    e!("Add from my apps", "Gaming", "add installed app as game"),
    e!("Add a game file", "Gaming", "add game manually custom program exe appimage"),
    // Accounts
    e!("Change password", "Accounts", "new password"),
    e!("Account picture", "Accounts", "avatar profile photo"),
    e!("Sign-in options", "Accounts", "login password lock"),
    e!("Require password after sleep", "Accounts", "lock wake"),
    e!("Sign in automatically", "Accounts", "autologin login"),
    e!("Add a user", "Accounts", "new account"),
    e!("Remove user", "Accounts", "delete account"),
    e!("Other users", "Accounts", "accounts list"),
    // Keyboard, mouse, default apps
    e!("Keyboard layouts", "Keyboard", "language layout input switch"),
    e!("Repeat rate", "Keyboard", "key repeat speed"),
    e!("Repeat delay", "Keyboard", "key repeat"),
    e!("Keyboard shortcuts", "Keyboard", "hotkeys keys"),
    e!("Double-click interval", "Mouse", "click speed"),
    e!("Web browser", "Default apps", "default browser chrome firefox brave"),
    e!("Email", "Default apps", "mail client"),
    e!("Music", "Default apps", "audio player"),
    e!("Video", "Default apps", "movies player"),
    e!("Photos", "Default apps", "images viewer"),
    e!("PDF documents", "Default apps", "pdf reader"),
    e!("Text files", "Default apps", "editor"),
    e!("Archives", "Default apps", "zip rar tar"),
    e!("Terminal", "Default apps", "console shell"),
    e!("Other file types", "Default apps", "extension open with"),
    // Troubleshoot, About
    e!("Report a problem", "Troubleshoot", "bug log feedback"),
    e!("System health", "Troubleshoot", "check problems"),
    e!("Save a problem report", "Troubleshoot", "report log usb"),
    e!("Open the log folder", "Troubleshoot", "logs"),
    e!("Check for problems in the background", "Troubleshoot", "health monitor"),
    e!("Device name", "About", "hostname rename computer"),
    e!("Operating system", "About", "os version arch"),
    e!("Kernel", "About", "linux version"),
    e!("Uptime", "About", "running time"),
    e!("Open full desktop settings", "About", "kde plasma system settings"),
];

// ── The results popover under the search box ────────────────────────────────────────────────────────────────────

/// Show matching settings under `entry` as the user types. `icon_of` gives a page's icon name; `on_pick` is called when a
/// result is chosen (click, or Enter / arrow keys), with the entry that was picked.
pub fn attach(
    entry: &gtk4::SearchEntry,
    icon_of: impl Fn(&str) -> &'static str + 'static,
    on_pick: impl Fn(&'static Entry) + 'static,
) {
    use std::cell::RefCell;
    use std::rc::Rc;

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    list.add_css_class("search-results");
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(420)
        .child(&list)
        .build();
    let popover = gtk4::Popover::builder().child(&scroll).has_arrow(false).autohide(false).can_focus(false).position(gtk4::PositionType::Bottom).build();
    popover.add_css_class("search-popover");
    popover.set_parent(entry);

    // The entry of every shown row, in order, so a row index maps back to what to open.
    let shown: Rc<RefCell<Vec<&'static Entry>>> = Rc::new(RefCell::new(Vec::new()));
    let on_pick = Rc::new(on_pick);

    let prev_rows = Rc::new(std::cell::Cell::new(0u32));
    let refresh = {
        let (list, popover, shown, entry, prev_rows) = (list.clone(), popover.clone(), shown.clone(), entry.clone(), prev_rows.clone());
        move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let q = entry.text().to_string();
            let hits = find(ENTRIES, &q, 8);
            if q.trim().is_empty() {
                shown.borrow_mut().clear();
                popover.popdown();
                return;
            }
            popover.set_size_request(entry.width().max(360), -1);
            if hits.is_empty() {
                shown.borrow_mut().clear();
                let l = gtk4::Label::builder().label(format!("No settings found for “{}”", q.trim())).halign(gtk4::Align::Start).margin_top(12).margin_bottom(12).margin_start(14).margin_end(14).build();
                l.add_css_class("dim-label");
                let row = gtk4::ListBoxRow::builder().child(&l).selectable(false).activatable(false).build();
                list.append(&row);
            } else {
                for h in &hits {
                    let b = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
                    b.set_margin_top(8);
                    b.set_margin_bottom(8);
                    b.set_margin_start(12);
                    b.set_margin_end(12);
                    let icon = gtk4::Image::from_icon_name(icon_of(h.page));
                    icon.set_pixel_size(20);
                    let text = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                    let title = gtk4::Label::builder().label(h.title).halign(gtk4::Align::Start).xalign(0.0).build();
                    // A page shows "Page"; a row shows where it lives, like Windows' "System › Display".
                    let sub = if h.title == h.page { "Settings page".to_string() } else { h.page.to_string() };
                    let subtitle = gtk4::Label::builder().label(sub).halign(gtk4::Align::Start).xalign(0.0).build();
                    subtitle.add_css_class("dim-label");
                    subtitle.add_css_class("caption");
                    text.append(&title);
                    text.append(&subtitle);
                    b.append(&icon);
                    b.append(&text);
                    list.append(&b_row(&b));
                }
                *shown.borrow_mut() = hits;
                list.select_row(list.row_at_index(0).as_ref());
            }
            // A popover keeps the height of the longest list it has shown; closing and reopening makes it fit the
            // new, shorter one (seen in the VM: one result under a tall empty box).
            if popover.is_visible() && prev_rows.get() > list.observe_children().n_items() {
                popover.popdown();
            }
            prev_rows.set(list.observe_children().n_items());
            popover.popup();
        }
    };

    let refresh: Rc<dyn Fn()> = Rc::new(refresh);
    entry.connect_search_changed({
        let refresh = refresh.clone();
        move |_| refresh()
    });

    // Click a result.
    list.connect_row_activated({
        let (shown, popover, on_pick, entry) = (shown.clone(), popover.clone(), on_pick.clone(), entry.clone());
        move |_, row| {
            let hit = shown.borrow().get(row.index() as usize).copied();
            if let Some(h) = hit {
                popover.popdown();
                entry.set_text("");
                on_pick(h);
            }
        }
    });

    // Enter picks the highlighted result.
    entry.connect_activate({
        let (list, shown, popover, on_pick) = (list.clone(), shown.clone(), popover.clone(), on_pick.clone());
        move |entry| {
            let idx = list.selected_row().map(|r| r.index()).unwrap_or(0) as usize;
            let hit = shown.borrow().get(idx).copied();
            if let Some(h) = hit {
                popover.popdown();
                entry.set_text("");
                on_pick(h);
            }
        }
    });

    // Up and Down move through the results while the cursor stays in the entry.
    let keys = gtk4::EventControllerKey::new();
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (list, popover) = (list.clone(), popover.clone());
        move |_, key, _, _| {
            if !popover.is_visible() {
                return glib::Propagation::Proceed;
            }
            let n = list.observe_children().n_items() as i32;
            let cur = list.selected_row().map(|r| r.index()).unwrap_or(-1);
            let next = match key {
                gtk4::gdk::Key::Down => Some((cur + 1).min(n - 1)),
                gtk4::gdk::Key::Up => Some((cur - 1).max(0)),
                _ => None,
            };
            match next {
                Some(i) => {
                    if let Some(r) = list.row_at_index(i) {
                        list.select_row(Some(&r));
                    }
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        }
    });
    entry.add_controller(keys);

    // Esc closes the list; leaving the box does too.
    entry.connect_stop_search({
        let popover = popover.clone();
        move |_| popover.popdown()
    });
    let focus = gtk4::EventControllerFocus::new();
    focus.connect_leave({
        let popover = popover.clone();
        move |_| {
            let p = popover.clone();
            // After the click on a result has been handled.
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || p.popdown());
        }
    });
    focus.connect_enter({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    entry.add_controller(focus);
}

fn b_row(child: &impl IsA<gtk4::Widget>) -> gtk4::ListBoxRow {
    gtk4::ListBoxRow::builder().child(child).build()
}

// ── Opening the result ──────────────────────────────────────────────────────────────────────────────────────────

/// Scroll the page so the row called `title` is in the middle, and flash it once. The page may still be filling in
/// (rows that load in the background), so it tries a few times.
pub fn reveal(page: &gtk4::Widget, title: &str) {
    reveal_try(page.clone(), normalize(title), 0);
}

fn reveal_try(page: gtk4::Widget, wanted: String, attempt: u32) {
    let found = titles_in(&page).into_iter().find(|(t, w)| normalize(t) == wanted && w.is_mapped());
    match found {
        Some((_, w)) => {
            // Layout is only final a moment after the page appears.
            glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                scroll_to(&w);
                w.add_css_class("search-hit");
                let w2 = w.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(1800), move || w2.remove_css_class("search-hit"));
            });
        }
        None if attempt < 8 => {
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || reveal_try(page, wanted, attempt + 1));
        }
        None => {}
    }
}

fn scroll_to(w: &gtk4::Widget) {
    let Some(sw) = w.ancestor(gtk4::ScrolledWindow::static_type()).and_then(|a| a.downcast::<gtk4::ScrolledWindow>().ok()) else { return };
    let Some(content) = sw.child().and_then(|v| v.downcast::<gtk4::Viewport>().ok()).and_then(|v| v.child()) else { return };
    let Some(b) = w.compute_bounds(&content) else { return };
    let adj = sw.vadjustment();
    let target = b.y() as f64 - (adj.page_size() - b.height() as f64) / 2.0;
    adj.set_value(target.clamp(adj.lower(), (adj.upper() - adj.page_size()).max(adj.lower())));
}
