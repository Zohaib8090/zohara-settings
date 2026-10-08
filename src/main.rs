mod pages;
mod backend;
mod theme;
mod search;
mod sysupdate;

use gtk4::prelude::*;
use libadwaita as adw;

use std::rc::Rc;
use std::cell::RefCell;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

static TOKIO_RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn tokio_runtime() -> &'static Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to initialize Tokio runtime")
    })
}

// CSS is now loaded from data/win11.css via include_str! so the file can be
// edited with normal CSS tooling, and a future light-theme variant can be
// loaded conditionally. The macro embeds the file contents at compile time,
// so no runtime path lookup is needed.
const WIN11_CSS: &str = include_str!("../data/win11.css");

/// Page requested with `--page <label>` (used by the health-check notification).
static START_PAGE: OnceLock<String> = OnceLock::new();

fn main() -> glib::ExitCode {
    backend::diag::init();
    let args: Vec<String> = std::env::args().collect();

    // Background health check (systemd user timer): no window, just notify.
    if args.iter().any(|a| a == "--health-check") {
        std::process::exit(backend::health::background_check());
    }
    // Machine-readable health report for the update pipeline (Store, VM tests, canary).
    if args.iter().any(|a| a == "--health-json") {
        std::process::exit(backend::health::json_report());
    }
    // Tray indicators for microphone / camera / location use: no window.
    if args.iter().any(|a| a == "--privacy-indicator") {
        std::process::exit(backend::privacy_indicator::run());
    }
    // Voice typing (Meta+H): no window, just listen and type.
    if args.iter().any(|a| a == "--dictate") {
        std::process::exit(backend::dictation::run());
    }
    // Taskbar keyboard button: show or hide the on-screen keyboard, no window.
    if args.iter().any(|a| a == "--toggle-keyboard") {
        std::process::exit(backend::touch_keyboard::toggle());
    }
    // First-login setup on a touch screen (autostart): turn on the keyboard and add its taskbar button.
    if args.iter().any(|a| a == "--touch-setup") {
        std::process::exit(backend::touch_keyboard::touch_setup());
    }
    // KDE Wallet starts off (autostart, once).
    if args.iter().any(|a| a == "--wallet-default") {
        std::process::exit(backend::wallet::apply_default());
    }
    // The Zohara logo on the start button (autostart, once).
    if args.iter().any(|a| a == "--branding") {
        std::process::exit(backend::branding::run_once());
    }
    // Automatic time zone (a systemd user timer runs this): find the zone from the internet connection and set it.
    if args.iter().any(|a| a == "--auto-timezone") {
        std::process::exit(backend::autotz::run_once());
    }
    // Background check for system updates (a systemd user timer runs this): a desktop notification, no window.
    if args.iter().any(|a| a == "--check-updates") {
        if pages::updates::active_pause_message().is_none() {
            let found = sysupdate::updates::check_all();
            sysupdate::updates::notify_pending(&found);
        }
        std::process::exit(0);
    }
    // Root helper for the system update (run through pkexec by the Zohara Update page): checks the signed approval
    // list itself, then moves the package mirror to the approved date.
    if let Some(i) = args.iter().position(|a| a == "--pin-date") {
        let (Some(m), Some(s)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("usage: zohara-settings --pin-date MANIFEST SIGNATURE");
            std::process::exit(2);
        };
        match sysupdate::manifest::root_pin(m, s, env!("CARGO_PKG_VERSION")) {
            Ok(msg) => {
                println!("{msg}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    if let Some(page) = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)) {
        let _ = START_PAGE.set(page.clone());
    }

    let rt = tokio_runtime();
    let _rt_guard = rt.enter();

    let app = adw::Application::builder()
        .application_id("os.zohara.Settings")
        .build();

    app.connect_activate(build_ui);
    // Our own flags are handled above; don't let GTK reject them.
    let code = app.run_with_args(&args[..1]);
    log::info!("Zohara Settings exiting");
    code
}

// ΓöÇΓöÇ Page registry (single source of truth) ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

struct PageDef {
    label: &'static str,
    icon:  &'static str,
    /// Not shown as its own sidebar row. Still a real, fully routable page
    /// (goto() and build_page_inner() both key off PAGES, not the sidebar) --
    /// just reached from its parent page instead of the top level, the same
    /// way Mouse/Touchpad/Keyboard/Printers already only show up inside
    /// Bluetooth & devices, not the sidebar. Kept in PAGES at all rather
    /// than removed so nothing else has to change to reach them.
    hidden: bool,
}

static PAGES: &[PageDef] = &[
    PageDef { label: "Home",                 icon: "user-home-symbolic",                          hidden: false },
    PageDef { label: "System",               icon: "computer-symbolic",                           hidden: false },
    PageDef { label: "Bluetooth & devices",  icon: "bluetooth-symbolic",                           hidden: false },
    PageDef { label: "Network & internet",   icon: "network-wireless-symbolic",                    hidden: false },
    PageDef { label: "Personalization",      icon: "preferences-desktop-symbolic",                 hidden: false },
    PageDef { label: "Apps",                 icon: "application-x-executable-symbolic",            hidden: false },
    PageDef { label: "Accounts",             icon: "system-users-symbolic",                        hidden: false },
    PageDef { label: "Time & language",      icon: "preferences-system-time-symbolic",             hidden: false },
    PageDef { label: "Gaming",               icon: "applications-games-symbolic",                  hidden: false },
    PageDef { label: "Accessibility",        icon: "preferences-desktop-accessibility-symbolic",   hidden: false },
    PageDef { label: "Privacy & security",   icon: "security-high-symbolic",                       hidden: false },
    PageDef { label: "Zohara Update",        icon: "system-software-update-symbolic",              hidden: false },
    // Reached from System's "Settings" group (system_links_group in system.rs).
    PageDef { label: "Display",              icon: "video-display-symbolic",                       hidden: true },
    PageDef { label: "Sound",                icon: "audio-speakers-symbolic",                      hidden: true },
    PageDef { label: "Notifications",        icon: "preferences-system-notifications-symbolic",    hidden: true },
    PageDef { label: "Power & battery",      icon: "battery-level-80-symbolic",                    hidden: true },
    PageDef { label: "Storage",              icon: "drive-harddisk-symbolic",                      hidden: true },
    // Reached from Bluetooth & devices' "Other devices" group (already existed).
    PageDef { label: "Mouse",                icon: "input-mouse-symbolic",                         hidden: true },
    PageDef { label: "Touchpad",             icon: "input-touchpad-symbolic",                      hidden: true },
    PageDef { label: "Keyboard",             icon: "input-keyboard-symbolic",                      hidden: true },
    PageDef { label: "Printers",             icon: "printer-symbolic",                             hidden: true },
    // Reached from Apps' own "Default apps" row (already existed).
    PageDef { label: "Default apps",         icon: "preferences-desktop-default-applications-symbolic", hidden: true },
    PageDef { label: "About",                icon: "help-about-symbolic",                          hidden: false },
    PageDef { label: "Zohara Link",          icon: "phone-symbolic",                               hidden: false },
    PageDef { label: "Troubleshoot",         icon: "system-help-symbolic",                         hidden: false },
];

/// Build a page, isolating failures: a page that panics while being built
/// shows an explanation (and is logged with a crash report) instead of
/// closing Settings.
fn build_page(index: usize) -> gtk4::Widget {
    let label = PAGES[index].label;
    backend::diag::set_current_page(label);
    match backend::diag::guard(label, || build_page_inner(index)) {
        Ok(w) => {
            let moved = pages::adopt_orphan_rows(&w);
            if moved > 0 {
                log::info!("{label}: {moved} row(s) were outside a list and were moved into one");
            }
            w
        }
        Err(msg) => {
            let details = gtk4::Button::with_label("Open Troubleshoot");
            details.add_css_class("pill");
            details.set_halign(gtk4::Align::Center);
            details.connect_clicked(|b| pages::goto(b, "Troubleshoot"));
            adw::StatusPage::builder()
                .icon_name("dialog-error-symbolic")
                .title(format!("{label} couldn't be opened"))
                .description(format!(
                    "Something went wrong while loading this page, so it was stopped to keep the rest of Settings working.

{}",
                    glib::markup_escape_text(&msg)
                ))
                .child(&details)
                .build()
                .upcast()
        }
    }
}

fn build_page_inner(index: usize) -> gtk4::Widget {
    match index {
        0  => pages::home::build(),
        1  => pages::system::build(),
        2  => pages::bluetooth::build(),
        3  => pages::network::build(),
        4  => pages::personalization::build(),
        5  => pages::apps::build(),
        6  => pages::accounts::build(),
        7  => pages::time_language::build(),
        8  => pages::gaming::build(),
        9  => pages::accessibility::build(),
        10 => pages::privacy::build(),
        11 => pages::updates::build(),
        12 => pages::display::build(),
        13 => pages::sound::build(),
        14 => pages::notifications::build(),
        15 => pages::power::build(),
        16 => pages::storage::build(),
        17 => pages::mouse::build(),
        18 => pages::touchpad::build(),
        19 => pages::keyboard::build(),
        20 => pages::printers::build(),
        21 => pages::default_apps::build(),
        22 => pages::advanced::build(),
        23 => pages::zohara_link::build(),
        24 => pages::troubleshoot::build(),
        _  => unreachable!("Page index {} out of range", index),
    }
}

// ΓöÇΓöÇ UI ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ

fn build_ui(app: &adw::Application) {
    if let Some(settings) = gtk4::Settings::default() {
        settings.set_gtk_decoration_layout(Some("icon:minimize,maximize,close"));
        // Respect the system light/dark preference instead of forcing dark.
        // The previous behaviour called set_gtk_application_prefer_dark_theme(true)
        // here, which made the Settings app ignore the user's Color mode toggle
        // on the Personalization page. The CSS in data/win11.css is dark-only,
        // so a future light theme would need to be loaded conditionally.
    }

    // Load the static stylesheet first (it declares fallback accent/background
    // named colors), then the theme engine's provider on top of it — that one
    // is registered at a higher priority, so the user's Personalization
    // choices (persisted in ~/.config/zohara/theme.json) win.
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(WIN11_CSS);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        theme::apply(&display);
    }

    let page_cache: Rc<RefCell<Vec<Option<gtk4::Widget>>>> =
        Rc::new(RefCell::new(vec![None; PAGES.len()]));

    // ΓöÇΓöÇ Main Layout Split ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    let root_h_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    // ΓöÇΓöÇ Left Navigation Sidebar ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_box.set_css_classes(&["win11-sidebar"]);
    sidebar_box.set_size_request(260, -1);

    // 1. User Profile Header Card
    let user_name = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string());
    let user_card = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    user_card.set_css_classes(&["win11-user-card"]);

    let avatar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    avatar_box.set_css_classes(&["win11-avatar-circle"]);
    let avatar_icon = gtk4::Image::from_icon_name("avatar-default-symbolic");
    avatar_icon.set_pixel_size(26);
    avatar_icon.set_valign(gtk4::Align::Center);
    avatar_icon.set_halign(gtk4::Align::Center);
    avatar_box.append(&avatar_icon);
    user_card.append(&avatar_box);

    let user_texts = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
    user_texts.set_valign(gtk4::Align::Center);
    let u_name = gtk4::Label::builder()
        .label(&user_name)
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-user-name".to_string()])
        .build();
    let u_email = gtk4::Label::builder()
        .label("Local account")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-user-email".to_string()])
        .build();
    user_texts.append(&u_name);
    user_texts.append(&u_email);
    user_card.append(&user_texts);
    sidebar_box.append(&user_card);

    // 2. Navigation Items List
    let nav_list = gtk4::ListBox::new();
    nav_list.set_css_classes(&["win11-nav-list"]);
    nav_list.set_selection_mode(gtk4::SelectionMode::Single);

    // Home..About is the main settings list; Zohara Link and Troubleshoot
    // are tools rather than settings, so they sit in their own section at
    // the bottom behind a divider instead of reading as more of the list.
    nav_list.set_header_func(|row, _before| {
        let starts_tools = row
            .widget_name()
            .parse::<usize>()
            .ok()
            .and_then(|i| PAGES.get(i))
            .map(|p| p.label == "Zohara Link")
            .unwrap_or(false);
        if starts_tools {
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            sep.set_css_classes(&["win11-nav-separator"]);
            row.set_header(Some(&sep));
        } else {
            row.set_header(None::<&gtk4::Widget>);
        }
    });

    for (i, page_def) in PAGES.iter().enumerate() {
        // Not a sidebar row -- still fully routable (see goto()/build_page(),
        // both keyed off PAGES directly), just reached from its parent page.
        if page_def.hidden {
            continue;
        }

        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        let icon = gtk4::Image::from_icon_name(page_def.icon);
        icon.set_pixel_size(18);

        // Icon accent color styling. Matched on the label, not position: a
        // match on `i` silently ran out at index 11 (Zohara Update) and left
        // every later row with no color class at all -- exactly the
        // "some icons colorful, some plain white" inconsistency reported on
        // real hardware. Every *visible* row gets an explicit class now.
        let accent = match page_def.label {
            "Home" => "accent-orange",
            "System" => "accent-blue",
            "Bluetooth & devices" => "accent-blue",
            "Network & internet" => "accent-blue",
            "Personalization" => "accent-orange",
            "Apps" => "accent-blue",
            "Accounts" => "accent-green",
            "Time & language" => "accent-blue",
            "Gaming" => "accent-purple",
            "Accessibility" => "accent-blue",
            "Privacy & security" => "accent-blue",
            "Zohara Update" => "accent-cyan",
            "About" => "accent-blue",
            "Zohara Link" => "accent-green",
            "Troubleshoot" => "accent-orange",
            _ => "accent-blue",
        };
        icon.set_css_classes(&[accent]);

        let lbl = gtk4::Label::builder()
            .label(page_def.label)
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();

        row_box.append(&icon);
        row_box.append(&lbl);

        let row = gtk4::ListBoxRow::builder()
            .child(&row_box)
            .build();
        row.set_widget_name(&i.to_string());
        nav_list.append(&row);
    }

    let nav_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .child(&nav_list)
        .build();
    sidebar_box.append(&nav_scroll);
    root_h_box.append(&sidebar_box);

    // ΓöÇΓöÇ Right Main Content Area ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    let main_content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    main_content_box.set_hexpand(true);
    main_content_box.set_vexpand(true);

    // Centered Top Header Bar with Windows 11 Pill Search Bar
    let header = adw::HeaderBar::new();

    let search_entry = gtk4::SearchEntry::builder()
        .placeholder_text("Find a setting")
        .css_classes(vec!["win11-search".to_string()])
        .build();
    header.set_title_widget(Some(&search_entry));
    main_content_box.append(&header);

    // Content container
    let content_stack = gtk4::Stack::new();
    content_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    content_stack.set_transition_duration(150);
    content_stack.set_vexpand(true);
    content_stack.set_hexpand(true);

    // Load initial page (Home, index 0)
    let home_widget = build_page(0);
    page_cache.borrow_mut()[0] = Some(home_widget.clone());
    content_stack.add_named(&home_widget, Some("page_0"));
    content_stack.set_visible_child_name("page_0");

    main_content_box.append(&content_stack);
    root_h_box.append(&main_content_box);

    // "Find a setting": results for single options, not only pages. Choosing one opens its page and flashes the row.
    {
        let stack = content_stack.clone();
        let entry = search_entry.clone();
        search::attach(
            &search_entry,
            |page| PAGES.iter().find(|p| p.label == page).map(|p| p.icon).unwrap_or("preferences-system-symbolic"),
            move |hit| {
                pages::goto(&entry, hit.page);
                if hit.title != hit.page {
                    let stack = stack.clone();
                    let title = hit.title;
                    // The page is built by now; give GTK a moment to show it before looking for the row.
                    glib::idle_add_local_once(move || {
                        if let Some(page) = stack.visible_child() {
                            search::reveal(&page, title);
                        }
                    });
                }
            },
        );
    }

    // ΓöÇΓöÇ Row Navigation Switching ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    let cache = page_cache.clone();
    let stack_clone = content_stack.clone();
    nav_list.connect_row_activated(move |_, row| {
        let idx: usize = match row.widget_name().parse() {
            Ok(i) if i < PAGES.len() => i,
            _ => return,
        };

        let page_tag = format!("page_{}", idx);
        let mut cache = cache.borrow_mut();
        if cache[idx].is_none() {
            let widget = build_page(idx);
            stack_clone.add_named(&widget, Some(&page_tag));
            cache[idx] = Some(widget);
        }
        stack_clone.set_visible_child_name(&page_tag);
    });

    if let Some(first_row) = nav_list.row_at_index(0) {
        nav_list.select_row(Some(&first_row));
    }

    // ΓöÇΓöÇ Window Setup ΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇΓöÇ
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Settings")
        .default_width(1120)
        .default_height(760)
        .content(&root_h_box)
        .css_classes(vec!["win11-window".to_string()])
        .build();

    // Lets any page open another one: `pages::goto(widget, "Mouse")`.
    // idx is PAGES's own index, which no longer lines up with the sidebar
    // ListBox's row positions now that hidden pages don't get a row at all
    // -- row_at_index(idx) would silently jump to the wrong page for
    // anything after a hidden one. Rows still carry their true PAGES index
    // in widget_name (set when built above), so look a matching row up by
    // that instead of assuming position; a hidden target has no row to
    // select, so just switch the content directly.
    let nav_for_goto = nav_list.clone();
    let cache_for_goto = page_cache.clone();
    let stack_for_goto = content_stack.clone();
    let goto = gtk4::gio::SimpleAction::new("goto", Some(glib::VariantTy::STRING));
    goto.connect_activate(move |_, param| {
        let Some(label) = param.and_then(|p| p.get::<String>()) else { return };
        let Some(idx) = PAGES.iter().position(|p| p.label == label) else { return };

        let mut found_row = None;
        let mut child = nav_for_goto.first_child();
        while let Some(c) = child {
            if let Some(row) = c.downcast_ref::<gtk4::ListBoxRow>() {
                if row.widget_name() == idx.to_string() {
                    found_row = Some(row.clone());
                    break;
                }
            }
            child = c.next_sibling();
        }

        if let Some(row) = found_row {
            nav_for_goto.select_row(Some(&row));
            row.activate();
        } else {
            // Hidden page: no sidebar row to activate, switch directly.
            let page_tag = format!("page_{}", idx);
            let mut cache = cache_for_goto.borrow_mut();
            if cache[idx].is_none() {
                let widget = build_page(idx);
                stack_for_goto.add_named(&widget, Some(&page_tag));
                cache[idx] = Some(widget);
            }
            stack_for_goto.set_visible_child_name(&page_tag);
            nav_for_goto.unselect_all();
        }
    });
    window.add_action(&goto);

    window.present();

    if let Some(page) = START_PAGE.get() {
        let _ = gtk4::prelude::WidgetExt::activate_action(&window, "win.goto", Some(&page.to_variant()));
    }

    // Offer the report if the previous run crashed.
    if let Some(report) = backend::diag::take_previous_crash() {
        use adw::prelude::*;
        let d = adw::AlertDialog::new(
            Some("Settings closed unexpectedly last time"),
            Some("A crash report was saved. You can review it and include it in a problem report from Troubleshoot."),
        );
        d.add_responses(&[("dismiss", "Dismiss"), ("open", "Open Troubleshoot")]);
        d.set_response_appearance("open", adw::ResponseAppearance::Suggested);
        let w = window.clone();
        d.connect_response(None, move |_, r| {
            if r == "open" {
                let _ = gtk4::prelude::WidgetExt::activate_action(&w, "win.goto", Some(&"Troubleshoot".to_variant()));
            }
        });
        d.present(Some(&window));
        log::info!("previous crash report: {}", report.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(q: &str) -> Vec<&'static str> {
        search::find(search::ENTRIES, q, 8).into_iter().map(|e| e.title).collect()
    }

    #[test]
    fn search_finds_single_options_not_just_pages() {
        // The cases from the request: "wifi" must lead to the Wi-Fi setting, however it is typed.
        assert_eq!(top("wifi")[0], "Wi-Fi");
        assert_eq!(top("Wi-Fi")[0], "Wi-Fi");
        assert_eq!(top("wi fi")[0], "Wi-Fi");
        assert_eq!(top("hotspot")[0], "Mobile hotspot");
        assert_eq!(top("bluetooth")[0], "Bluetooth");
        assert_eq!(top("firewall")[0], "Firewall");
        assert!(top("ethernet").contains(&"Network adapters"));
        // Words people use for a setting that is named differently on the page.
        assert!(top("screen timeout").contains(&"Turn off screen after"));
        assert!(top("wallpaper").contains(&"Background"));
        assert!(top("dark mode").contains(&"Choose your mode"));
        assert!(top("microphone").contains(&"Input device"));
        assert_eq!(top("check for updates")[0], "Zohara Update");
        assert!(top("go back").contains(&"Undo the last update"));
    }

    #[test]
    fn search_ignores_nothing_and_matches_nothing_for_nonsense() {
        assert!(top("").is_empty());
        assert!(top("   ").is_empty());
        assert!(top("qzxjvk").is_empty());
        // Every word must match: a second unrelated word removes the result.
        assert!(top("wifi qzxjvk").is_empty());
    }

    #[test]
    fn every_entry_points_at_a_real_page() {
        for e in search::ENTRIES {
            assert!(PAGES.iter().any(|p| p.label == e.page), "'{}' points at unknown page '{}'", e.title, e.page);
        }
        for p in PAGES {
            // Every page, including hidden ones (Display, Sound, ...), can itself be found.
            assert!(search::ENTRIES.iter().any(|e| e.title == p.label && e.page == p.label), "page '{}' cannot be found by search", p.label);
        }
    }

    #[test]
    fn every_entry_is_on_its_page() {
        // Drift guard: a row that was renamed in a page file but not in the index would open the page and highlight
        // nothing. The title must appear in the page sources.
        let mut all = String::new();
        for sub in ["src/pages", "src/sysupdate"] {
            for f in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(sub)).unwrap() {
                all.push_str(&std::fs::read_to_string(f.unwrap().path()).unwrap_or_default());
            }
        }
        let missing: Vec<&str> = search::ENTRIES.iter().filter(|e| e.title != e.page && !all.contains(&format!("\"{}\"", e.title))).map(|e| e.title).collect();
        assert!(missing.is_empty(), "in the search index but not in any page file: {missing:?}");
    }
}
