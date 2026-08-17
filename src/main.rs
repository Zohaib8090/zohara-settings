mod pages;
mod backend;

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
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

fn main() {
    let rt = tokio_runtime();
    let _rt_guard = rt.enter();

    let app = adw::Application::builder()
        .application_id("os.zohara.Settings")
        .build();

    app.connect_activate(build_ui);
    app.run();
}

// ── Page registry (single source of truth) ────────────────────────────────────

struct PageDef {
    label: &'static str,
    icon:  &'static str,
}

static PAGES: &[PageDef] = &[
    PageDef { label: "System",              icon: "computer-symbolic" },
    PageDef { label: "Network &amp; internet",  icon: "network-wireless-symbolic" },
    PageDef { label: "Bluetooth &amp; devices", icon: "bluetooth-symbolic" },
    PageDef { label: "Personalization",     icon: "preferences-desktop-symbolic" },
    PageDef { label: "Apps",               icon: "application-x-executable-symbolic" },
    PageDef { label: "Accounts",           icon: "system-users-symbolic" },
    PageDef { label: "Gaming",             icon: "applications-games-symbolic" },
    PageDef { label: "Time &amp; language",    icon: "preferences-system-time-symbolic" },
    PageDef { label: "Accessibility",      icon: "preferences-desktop-accessibility-symbolic" },
    PageDef { label: "Privacy &amp; security", icon: "security-high-symbolic" },
    PageDef { label: "Zohara Update",      icon: "system-software-update-symbolic" },
    PageDef { label: "Advanced (KDE)",     icon: "configure-symbolic" },
];

/// Build a page widget by its index into PAGES.
/// This is the only place that maps index → page builder.
fn build_page(index: usize) -> gtk4::Widget {
    match index {
        0  => pages::system::build(),
        1  => pages::network::build(),
        2  => pages::bluetooth::build(),
        3  => pages::personalization::build(),
        4  => pages::apps::build(),
        5  => pages::accounts::build(),
        6  => pages::gaming::build(),
        7  => pages::time_language::build(),
        8  => pages::accessibility::build(),
        9  => pages::privacy::build(),
        10 => pages::updates::build(),
        11 => pages::advanced::build(),
        _  => unreachable!("Page index {} out of range", index),
    }
}

// ── UI ────────────────────────────────────────────────────────────────────────

fn build_ui(app: &adw::Application) {
    if let Some(settings) = gtk4::Settings::default() {
        settings.set_gtk_decoration_layout(Some("icon:minimize,maximize,close"));
    }

    // Lazy page cache: each page is built once on first visit and reused on
    // subsequent visits, preventing redundant re-scans (Bluetooth, Network, Apps).
    let page_cache: Rc<RefCell<Vec<Option<gtk4::Widget>>>> =
        Rc::new(RefCell::new(vec![None; PAGES.len()]));

    // ── Navigation split-view ─────────────────────────────────────────────────
    let nav_split = adw::NavigationSplitView::new();
    nav_split.set_min_sidebar_width(240.0);
    nav_split.set_max_sidebar_width(280.0);

    // ── Sidebar ───────────────────────────────────────────────────────────────
    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.set_show_end_title_buttons(false);
    let header_label = gtk4::Label::new(Some("Zohara Settings"));
    header_label.set_css_classes(&["heading"]);
    sidebar_header.set_title_widget(Some(&header_label));

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Find a setting"));
    search_entry.set_margin_start(12);
    search_entry.set_margin_end(12);
    search_entry.set_margin_top(8);
    search_entry.set_margin_bottom(8);

    let nav_list = gtk4::ListBox::new();
    nav_list.set_css_classes(&["navigation-sidebar"]);
    nav_list.set_selection_mode(gtk4::SelectionMode::Single);

    // Search filter — match against the ActionRow title text
    let search_clone = search_entry.clone();
    nav_list.set_filter_func(move |row| {
        let query = search_clone.text().to_lowercase();
        if query.is_empty() { return true; }
        row.downcast_ref::<adw::ActionRow>()
            .map(|r| r.title().to_lowercase().contains(&query))
            .unwrap_or(true)
    });
    let nav_list_for_search = nav_list.clone();
    search_entry.connect_search_changed(move |_| nav_list_for_search.invalidate_filter());

    // Build nav rows. The page index is stored as the widget name so that
    // connect_row_activated can look it up reliably (immune to search filtering).
    for (i, page_def) in PAGES.iter().enumerate() {
        let row = adw::ActionRow::builder()
            .title(page_def.label)  // plain &str — no HTML entities needed
            .activatable(true)
            .build();
        row.set_widget_name(&i.to_string()); // index, not label, for O(1) lookup

        row.add_prefix(&gtk4::Image::from_icon_name(page_def.icon));
        let chevron = gtk4::Image::from_icon_name("go-next-symbolic");
        chevron.set_css_classes(&["dim-label"]);
        row.add_suffix(&chevron);

        nav_list.append(&row);
    }

    let scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .child(&nav_list)
        .build();

    sidebar_box.append(&sidebar_header);
    sidebar_box.append(&search_entry);
    sidebar_box.append(&scroll);

    let sidebar_nav_page = adw::NavigationPage::builder()
        .title("Settings")
        .build();
    sidebar_nav_page.set_child(Some(&sidebar_box));
    nav_split.set_sidebar(Some(&sidebar_nav_page));

    // ── Content pane ──────────────────────────────────────────────────────────
    // A single NavigationPage whose child is swapped on nav selection.
    let content_page = adw::NavigationPage::builder()
        .title("System")
        .build();

    // Build and show the System page immediately (index 0)
    {
        let system_widget = build_page(0);
        page_cache.borrow_mut()[0] = Some(system_widget.clone());
        content_page.set_child(Some(&system_widget));
    }

    // NavigationSplitView requires its content to be a NavigationPage containing
    // a NavigationView — this satisfies that contract.
    let content_nav = adw::NavigationView::new();
    content_nav.push(&content_page);

    let content_wrapper = adw::NavigationPage::builder()
        .title("Settings")
        .build();
    content_wrapper.set_child(Some(&content_nav));
    nav_split.set_content(Some(&content_wrapper));

    // ── Navigation handler ────────────────────────────────────────────────────
    let cache = page_cache.clone();
    nav_list.connect_row_activated(move |_, row| {
        // Parse the index stored in widget_name — safe even when rows are filtered
        let idx: usize = match row.widget_name().parse() {
            Ok(i) if i < PAGES.len() => i,
            _ => return,
        };

        let mut cache = cache.borrow_mut();
        if cache[idx].is_none() {
            cache[idx] = Some(build_page(idx));
        }
        if let Some(ref widget) = cache[idx] {
            content_page.set_title(PAGES[idx].label);
            content_page.set_child(Some(widget));
        }
    });

    // Select the first row (System) on startup
    if let Some(first_row) = nav_list.row_at_index(0) {
        nav_list.select_row(Some(&first_row));
    }

    // ── Window ────────────────────────────────────────────────────────────────
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Zohara Settings")
        .default_width(1060)
        .default_height(740)
        .content(&nav_split)
        .build();

    window.present();
}
