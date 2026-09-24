//! Printers: the installed CUPS printers with their status, default printer,
//! paper size / two-sided / colour / quality options, waiting documents, test
//! page, pause and remove; plus adding driverless (IPP Everywhere) printers
//! found on the network or typed in by address.
//!
//! Reads with `lpstat`/`lpoptions` as the user. Changes to the system's
//! printers (add, remove, pause) run `lpadmin`/`cupsenable` through pkexec.
//! Options and the default printer are per user (`~/.cups/lpoptions`), like
//! on Windows. Network printers are found with `avahi-browse`.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

const TEST_PAGE: &str = "/usr/share/cups/data/testprint";
const CUPS_ADMIN: &str = "http://localhost:631/admin";

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).env("LC_ALL", "C").output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn run_ok(cmd: &str, args: &[&str]) -> Result<(), String> {
    let o = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
    Err(if e.is_empty() { format!("{cmd} failed") } else { e })
}

/// Runs a change to the system's printers as administrator.
/// `Ok(false)` means the password prompt was dismissed.
fn admin(cmd: &str, args: &[&str]) -> Result<bool, String> {
    let o = Command::new("pkexec").arg(cmd).args(args).output().map_err(|e| format!("pkexec: {e}"))?;
    match o.status.code() {
        Some(0) => Ok(true),
        // pkexec: 126 = dismissed, 127 = not authorized
        Some(126) | Some(127) => Ok(false),
        _ => {
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(if e.is_empty() { format!("{cmd} failed") } else { e })
        }
    }
}

// ── Installed printers ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Default, PartialEq)]
enum State {
    #[default]
    Ready,
    Printing,
    Paused,
}

#[derive(Clone)]
struct Job {
    id: String,
    user: String,
    size: u64,
}

#[derive(Clone)]
struct Choice {
    key: String,
    title: &'static str,
    values: Vec<String>,
    current: usize,
}

#[derive(Clone, Default)]
struct Printer {
    name: String,
    description: String,
    location: String,
    uri: String,
    state: State,
    message: String,
    default: bool,
    jobs: Vec<Job>,
    options: Vec<Choice>,
}

impl Printer {
    fn title(&self) -> &str {
        if self.description.is_empty() {
            &self.name
        } else {
            &self.description
        }
    }

    fn status(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.default {
            parts.push("Default".into());
        }
        parts.push(
            match self.state {
                State::Ready => "Ready",
                State::Printing => "Printing",
                State::Paused => "Paused",
            }
            .into(),
        );
        match self.jobs.len() {
            0 => {}
            1 => parts.push("1 document waiting".into()),
            n => parts.push(format!("{n} documents waiting")),
        }
        parts.join(" · ")
    }
}

struct Snapshot {
    running: bool,
    printers: Vec<Printer>,
}

/// `lpstat -l -p`: a "printer NAME ..." line per printer, then indented
/// details. The first indented line is the printer's status message, if any.
fn parse_printers(text: &str) -> Vec<Printer> {
    let mut out: Vec<Printer> = Vec::new();
    let mut first = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("printer ") {
            let state = if rest.contains(" disabled since") {
                State::Paused
            } else if rest.contains(" now printing ") {
                State::Printing
            } else {
                State::Ready
            };
            out.push(Printer {
                name: rest.split_whitespace().next().unwrap_or_default().to_string(),
                state,
                ..Default::default()
            });
            first = true;
            continue;
        }
        let Some(p) = out.last_mut() else { continue };
        let t = line.trim();
        if first && !t.is_empty() && !t.ends_with(':') && !t.starts_with("Form mounts") {
            p.message = t.to_string();
        } else if let Some(v) = t.strip_prefix("Description:") {
            p.description = v.trim().to_string();
        } else if let Some(v) = t.strip_prefix("Location:") {
            p.location = v.trim().to_string();
        }
        first = false;
    }
    out
}

/// `lpstat -o`: "QUEUE-ID  user  size  date".
fn parse_jobs(text: &str, printer: &str) -> Vec<Job> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let id = f.next()?;
            let (queue, _) = id.rsplit_once('-')?;
            (queue == printer).then(|| Job {
                id: id.to_string(),
                user: f.next().unwrap_or_default().to_string(),
                size: f.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

/// The options worth showing, by their PPD and IPP names.
const OPTIONS: &[(&[&str], &str)] = &[
    (&["PageSize", "media"], "Paper size"),
    (&["Duplex", "sides"], "Print on both sides"),
    (&["ColorModel", "print-color-mode"], "Color"),
    (&["cupsPrintQuality", "print-quality"], "Quality"),
];

/// `lpoptions -l`: "PageSize/Media Size: Letter *A4 Legal", `*` marking the current value.
fn options(printer: &str) -> Vec<Choice> {
    let text = run("lpoptions", &["-p", printer, "-l"]).unwrap_or_default();
    let key_of = |l: &str| l.split(['/', ':']).next().unwrap_or_default().to_string();
    let mut out = Vec::new();
    for (keys, title) in OPTIONS {
        let Some(line) = text.lines().find(|l| keys.contains(&key_of(l).as_str())) else { continue };
        let Some((_, vals)) = line.split_once(": ") else { continue };
        let mut current = 0;
        let mut values = Vec::new();
        for (i, v) in vals.split_whitespace().enumerate() {
            if let Some(v) = v.strip_prefix('*') {
                current = i;
                values.push(v.to_string());
            } else {
                values.push(v.to_string());
            }
        }
        if values.len() > 1 {
            out.push(Choice { key: key_of(line), title: *title, values, current });
        }
    }
    out
}

fn pretty(v: &str) -> String {
    match v {
        "None" | "one-sided" => "Off".into(),
        "DuplexNoTumble" | "two-sided-long-edge" => "Flip on long edge".into(),
        "DuplexTumble" | "two-sided-short-edge" => "Flip on short edge".into(),
        "Gray" | "Grayscale" | "monochrome" => "Black and white".into(),
        "RGB" | "CMYK" | "color" => "Color".into(),
        _ => v.replace('_', " "),
    }
}

fn load() -> Snapshot {
    let running = run("lpstat", &["-r"]).is_some_and(|s| s.contains("scheduler is running"));
    if !running {
        return Snapshot { running, printers: Vec::new() };
    }
    let mut printers = parse_printers(&run("lpstat", &["-l", "-p"]).unwrap_or_default());
    // Honours the user's own default (lpoptions -d) over the system's.
    let default = run("lpstat", &["-d"]).and_then(|s| s.split_once(": ").map(|(_, d)| d.trim().to_string()));
    let uris = run("lpstat", &["-v"]).unwrap_or_default();
    let jobs = run("lpstat", &["-o"]).unwrap_or_default();
    for p in &mut printers {
        p.default = default.as_deref() == Some(p.name.as_str());
        let prefix = format!("device for {}: ", p.name);
        p.uri = uris.lines().find_map(|l| l.strip_prefix(prefix.as_str())).unwrap_or_default().to_string();
        p.jobs = parse_jobs(&jobs, &p.name);
        p.options = options(&p.name);
    }
    Snapshot { running, printers }
}

// ── Network discovery ──────────────────────────────────────────────────────

#[derive(Clone)]
struct Found {
    name: String,
    model: String,
    location: String,
    uri: String,
}

/// Undoes avahi's escaping: `\032` is a decimal byte, `\.` a literal dot.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
            let n = (b[i + 1] - b'0') as u32 * 100 + (b[i + 2] - b'0') as u32 * 10 + (b[i + 3] - b'0') as u32;
            out.push(n.min(255) as u8);
            i += 4;
        } else if b[i] == b'\\' && i + 1 < b.len() {
            out.push(b[i + 1]);
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A key from avahi's TXT field: `"txtvers=1" "rp=ipp/print" "ty=HP LaserJet"`.
fn txt_value(txt: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    txt.trim()
        .trim_matches('"')
        .split("\" \"")
        .find_map(|kv| kv.strip_prefix(prefix.as_str()))
        .map(unescape)
        .filter(|v| !v.is_empty())
}

fn can_discover() -> bool {
    Command::new("avahi-browse").arg("--version").output().is_ok()
}

/// Driverless printers on the local network that aren't added yet.
fn discover(installed: &[Printer]) -> Vec<Found> {
    let own = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim().to_lowercase();
    let mut out: Vec<Found> = Vec::new();
    for (service, scheme) in [("_ipp._tcp", "ipp"), ("_ipps._tcp", "ipps")] {
        let text = run("avahi-browse", &["-rtp", service]).unwrap_or_default();
        // =;iface;proto;name;type;domain;host;address;port;txt
        for line in text.lines().filter(|l| l.starts_with("=;")) {
            let f: Vec<&str> = line.splitn(10, ';').collect();
            if f.len() < 10 {
                continue;
            }
            let name = unescape(f[3]);
            let host = f[6].to_string();
            if out.iter().any(|x| x.name == name) || host.to_lowercase().trim_end_matches(".local") == own {
                continue;
            }
            let encoded = name.replace(' ', "%20");
            if installed.iter().any(|p| p.uri.contains(&host) || p.uri.contains(&encoded)) {
                continue;
            }
            let rp = txt_value(f[9], "rp").unwrap_or_else(|| "ipp/print".into());
            out.push(Found {
                model: txt_value(f[9], "ty").unwrap_or_default(),
                location: txt_value(f[9], "note").unwrap_or_default(),
                uri: format!("{scheme}://{host}:{}/{rp}", f[8]),
                name,
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// A CUPS queue name (letters, digits, - and _) not already in use.
fn queue_name(display: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for c in display.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' };
        if !(c == '_' && base.ends_with('_')) {
            base.push(c);
        }
    }
    let mut base: String = base.trim_matches('_').chars().take(100).collect();
    if base.is_empty() {
        base = "Printer".into();
    }
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

/// Adds a driverless printer: CUPS asks the printer itself what it can do.
fn add_printer(queue: &str, uri: &str, description: &str, location: &str) -> Result<bool, String> {
    admin("lpadmin", &["-p", queue, "-E", "-v", uri, "-m", "everywhere", "-D", description, "-L", location])
}

// ── UI ─────────────────────────────────────────────────────────────────────

struct Ui {
    root: gtk4::Box,
    installed: gtk4::Box,
    found: adw::PreferencesGroup,
    found_rows: RefCell<Vec<gtk4::Widget>>,
    printers: RefCell<Vec<Printer>>,
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

/// Runs `work` off the UI thread with `button` disabled, then refreshes.
fn act(ui: &Rc<Ui>, button: &gtk4::Button, what: &'static str, work: impl FnOnce() -> Result<bool, String> + Send + 'static) {
    button.set_sensitive(false);
    let (ui, button) = (ui.clone(), button.clone());
    in_background(work, move |r| {
        button.set_sensitive(true);
        match r {
            Ok(true) => {
                refresh(&ui);
                refresh_found(&ui);
            }
            Ok(false) => {}
            Err(e) => {
                log::warn!("printers: {what}: {e}");
                message(&ui.root, what, &e);
            }
        }
    });
}

fn suffix_button(label: &str) -> gtk4::Button {
    let b = gtk4::Button::with_label(label);
    b.set_valign(gtk4::Align::Center);
    b
}

fn printer_row(ui: &Rc<Ui>, p: &Printer) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(&esc(p.title()));
    row.set_subtitle(&esc(&p.status()));
    row.add_prefix(&gtk4::Image::from_icon_name("printer-symbolic"));

    // Status, with pause / resume.
    let status = adw::ActionRow::new();
    status.set_title("Status");
    let mut sub = p.status();
    if !p.message.is_empty() {
        sub = format!("{sub}\n{}", p.message);
    }
    status.set_subtitle(&esc(&sub));
    let paused = p.state == State::Paused;
    let toggle = suffix_button(if paused { "Resume" } else { "Pause" });
    {
        let (ui2, name) = (ui.clone(), p.name.clone());
        toggle.connect_clicked(move |b| {
            let name = name.clone();
            let what = if paused { "Couldn't resume the printer" } else { "Couldn't pause the printer" };
            act(&ui2, b, what, move || {
                if paused {
                    admin("cupsenable", &[name.as_str()])
                } else {
                    admin("cupsdisable", &[name.as_str()])
                }
            });
        });
    }
    status.add_suffix(&toggle);
    row.add_row(&status);

    // Default printer (per user).
    let def = adw::ActionRow::new();
    def.set_title("Default printer");
    if p.default {
        def.set_subtitle("Apps print here unless you pick another printer");
        def.add_suffix(&gtk4::Image::from_icon_name("object-select-symbolic"));
    } else {
        def.set_subtitle("Print here unless you pick another printer");
        let set = suffix_button("Make default");
        let (ui2, name) = (ui.clone(), p.name.clone());
        set.connect_clicked(move |b| {
            let name = name.clone();
            act(&ui2, b, "Couldn't change the default printer", move || {
                run_ok("lpoptions", &["-d", name.as_str()]).map(|_| true)
            });
        });
        def.add_suffix(&set);
    }
    row.add_row(&def);

    // Printing options (per user).
    for c in &p.options {
        let combo = adw::ComboRow::new();
        combo.set_title(c.title);
        let labels: Vec<String> = c.values.iter().map(|v| pretty(v)).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        combo.set_model(Some(&gtk4::StringList::new(&refs)));
        combo.set_selected(c.current as u32);
        let (root, name, key, values) = (ui.root.clone(), p.name.clone(), c.key.clone(), c.values.clone());
        combo.connect_selected_notify(move |r| {
            let Some(v) = values.get(r.selected() as usize) else { return };
            let (name, opt, root) = (name.clone(), format!("{key}={v}"), root.clone());
            in_background(
                move || run_ok("lpoptions", &["-p", name.as_str(), "-o", opt.as_str()]),
                move |res| {
                    if let Err(e) = res {
                        message(&root, "Option not changed", &e);
                    }
                },
            );
        });
        row.add_row(&combo);
    }

    // Documents waiting.
    for j in &p.jobs {
        let jr = adw::ActionRow::new();
        let num = j.id.rsplit('-').next().unwrap_or(&j.id);
        jr.set_title(&format!("Document {num}"));
        jr.set_subtitle(&esc(&format!("{} · {} KB", j.user, (j.size + 1023) / 1024)));
        jr.add_prefix(&gtk4::Image::from_icon_name("document-print-symbolic"));
        let cancel = suffix_button("Cancel");
        let (ui2, id) = (ui.clone(), j.id.clone());
        cancel.connect_clicked(move |b| {
            let id = id.clone();
            act(&ui2, b, "Couldn't cancel the document", move || run_ok("cancel", &[id.as_str()]).map(|_| true));
        });
        jr.add_suffix(&cancel);
        row.add_row(&jr);
    }

    // Test page.
    let test = adw::ActionRow::new();
    test.set_title("Print a test page");
    test.set_subtitle("Check that the printer works");
    let print = suffix_button("Print");
    {
        let (ui2, name) = (ui.clone(), p.name.clone());
        print.connect_clicked(move |b| {
            let name = name.clone();
            act(&ui2, b, "Couldn't print a test page", move || {
                run_ok("lp", &["-d", name.as_str(), "-t", "Test page", TEST_PAGE]).map(|_| true)
            });
        });
    }
    test.add_suffix(&print);
    row.add_row(&test);

    // Connection and removal.
    let conn = adw::ActionRow::new();
    conn.set_title("Connection");
    let mut where_ = p.uri.clone();
    if !p.location.is_empty() {
        where_ = format!("{} · {where_}", p.location);
    }
    conn.set_subtitle(&esc(&where_));
    conn.set_subtitle_selectable(true);
    let remove = suffix_button("Remove");
    remove.add_css_class("destructive-action");
    {
        let (ui2, name, title) = (ui.clone(), p.name.clone(), p.title().to_string());
        remove.connect_clicked(move |b| {
            let d = adw::AlertDialog::new(
                Some(&format!("Remove {title}?")),
                Some("You can add it again later."),
            );
            d.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
            d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            let (ui3, name, b2) = (ui2.clone(), name.clone(), b.clone());
            d.connect_response(None, move |_, resp| {
                if resp != "remove" {
                    return;
                }
                let name = name.clone();
                act(&ui3, &b2, "Couldn't remove the printer", move || admin("lpadmin", &["-x", name.as_str()]));
            });
            d.present(Some(b));
        });
    }
    conn.add_suffix(&remove);
    row.add_row(&conn);
    row
}

fn render(ui: &Rc<Ui>, snap: Snapshot) {
    while let Some(c) = ui.installed.first_child() {
        ui.installed.remove(&c);
    }
    if !snap.running {
        let start = gtk4::Button::with_label("Start printing service");
        start.add_css_class("pill");
        start.add_css_class("suggested-action");
        start.set_halign(gtk4::Align::Center);
        let ui2 = ui.clone();
        start.connect_clicked(move |b| {
            act(&ui2, b, "Couldn't start the printing service", || {
                admin("systemctl", &["enable", "--now", "cups.socket", "cups.service"])
            });
        });
        ui.installed.append(
            &adw::StatusPage::builder()
                .icon_name("printer-symbolic")
                .title("The printing service isn't running")
                .description("Printing is handled by CUPS. Start it to add and use printers.")
                .child(&start)
                .build(),
        );
        *ui.printers.borrow_mut() = Vec::new();
        return;
    }

    let g = adw::PreferencesGroup::new();
    g.set_title("Printers");
    let reload = gtk4::Button::from_icon_name("view-refresh-symbolic");
    reload.add_css_class("flat");
    reload.set_tooltip_text(Some("Refresh"));
    let ui2 = ui.clone();
    reload.connect_clicked(move |_| refresh(&ui2));
    g.set_header_suffix(Some(&reload));
    if snap.printers.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("No printers yet");
        r.set_subtitle("Add one below. Most network printers made in the last ten years work without installing a driver");
        r.add_prefix(&gtk4::Image::from_icon_name("printer-symbolic"));
        g.add(&r);
    }
    for p in &snap.printers {
        g.add(&printer_row(ui, p));
    }
    ui.installed.append(&g);
    *ui.printers.borrow_mut() = snap.printers;
}

fn refresh(ui: &Rc<Ui>) {
    let ui = ui.clone();
    in_background(load, move |snap| render(&ui, snap));
}

fn set_found_rows(ui: &Rc<Ui>, rows: Vec<gtk4::Widget>) {
    for r in ui.found_rows.borrow_mut().drain(..) {
        ui.found.remove(&r);
    }
    for r in &rows {
        ui.found.add(r);
    }
    *ui.found_rows.borrow_mut() = rows;
}

fn info_row(title: &str, subtitle: &str) -> gtk4::Widget {
    let r = adw::ActionRow::new();
    r.set_title(title);
    r.set_subtitle(subtitle);
    r.upcast()
}

fn refresh_found(ui: &Rc<Ui>) {
    if !can_discover() {
        set_found_rows(ui, vec![info_row("Network search isn't available", "avahi is not installed")]);
        return;
    }
    let searching = adw::ActionRow::new();
    searching.set_title("Searching…");
    let spinner = gtk4::Spinner::new();
    spinner.start();
    searching.add_suffix(&spinner);
    set_found_rows(ui, vec![searching.upcast()]);

    let installed = ui.printers.borrow().clone();
    let ui = ui.clone();
    in_background(
        move || discover(&installed),
        move |found| {
            if found.is_empty() {
                set_found_rows(
                    &ui,
                    vec![info_row(
                        "No new printers found",
                        "Check that the printer is on and connected to the same network, or add it by address below",
                    )],
                );
                return;
            }
            let rows = found
                .into_iter()
                .map(|f| {
                    let r = adw::ActionRow::new();
                    r.set_title(&esc(&f.name));
                    let sub: Vec<&str> = [f.model.as_str(), f.location.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                    r.set_subtitle(&esc(&sub.join(" · ")));
                    r.add_prefix(&gtk4::Image::from_icon_name("printer-network-symbolic"));
                    let add = suffix_button("Add");
                    add.add_css_class("suggested-action");
                    let ui2 = ui.clone();
                    add.connect_clicked(move |b| {
                        let taken: Vec<String> = ui2.printers.borrow().iter().map(|p| p.name.clone()).collect();
                        let queue = queue_name(&f.name, &taken);
                        let (uri, desc, loc) = (f.uri.clone(), f.name.clone(), f.location.clone());
                        act(&ui2, b, "Couldn't add the printer", move || add_printer(&queue, &uri, &desc, &loc));
                    });
                    r.add_suffix(&add);
                    r.upcast::<gtk4::Widget>()
                })
                .collect();
            set_found_rows(&ui, rows);
        },
    );
}

fn address_group(ui: &Rc<Ui>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Add by address");
    g.set_description(Some("For a network printer that isn't found automatically. Its address is usually on the printer's screen or network settings page"));
    let entry = adw::EntryRow::new();
    entry.set_title("IP address or name, like 192.168.1.20");
    entry.set_show_apply_button(true);
    let ui2 = ui.clone();
    entry.connect_apply(move |e| {
        let host = e.text().trim().trim_start_matches("ipp://").trim_end_matches('/').to_string();
        if host.is_empty() || host.contains(char::is_whitespace) {
            message(&ui2.root, "Not a printer address", "Type the printer's IP address or network name.");
            return;
        }
        let taken: Vec<String> = ui2.printers.borrow().iter().map(|p| p.name.clone()).collect();
        let queue = queue_name(&host, &taken);
        let uri = if host.contains('/') { format!("ipp://{host}") } else { format!("ipp://{host}/ipp/print") };
        e.set_sensitive(false);
        let (ui3, e2) = (ui2.clone(), e.clone());
        in_background(
            move || add_printer(&queue, &uri, &host, ""),
            move |r| {
                e2.set_sensitive(true);
                match r {
                    Ok(true) => {
                        e2.set_text("");
                        refresh(&ui3);
                    }
                    Ok(false) => {}
                    Err(err) => message(
                        &ui3.root,
                        "Couldn't add the printer",
                        &format!("{err}\n\nCheck the address, and that the printer supports driverless printing (IPP Everywhere or AirPrint)."),
                    ),
                }
            },
        );
    });
    g.add(&entry);
    g
}

fn more_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("More");
    let r = adw::ActionRow::new();
    r.set_title("Printers that need a driver");
    r.set_subtitle("Set up USB and older printers in the CUPS printer manager. Some need a driver package from Zohara Store, such as hplip for HP");
    r.add_prefix(&gtk4::Image::from_icon_name("preferences-system-symbolic"));
    let open = suffix_button("Open");
    open.connect_clicked(|_| {
        let _ = Command::new("xdg-open").arg(CUPS_ADMIN).spawn();
    });
    r.add_suffix(&open);
    r.set_activatable_widget(Some(&open));
    g.add(&r);
    g
}

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
            .label("Printers")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    if Command::new("lpstat").arg("-r").output().is_err() {
        root.append(
            &adw::StatusPage::builder()
                .icon_name("printer-symbolic")
                .title("Printing isn't installed")
                .description("Install the cups package from Zohara Store to use printers.")
                .build(),
        );
        scroll.set_child(Some(&root));
        return scroll.upcast();
    }

    let installed = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.append(&installed);

    let found = adw::PreferencesGroup::new();
    found.set_title("Printers on your network");
    found.set_description(Some("Printers that work without installing a driver"));
    let rescan = gtk4::Button::from_icon_name("view-refresh-symbolic");
    rescan.add_css_class("flat");
    rescan.set_tooltip_text(Some("Search again"));
    found.set_header_suffix(Some(&rescan));
    root.append(&found);

    let ui = Rc::new(Ui {
        root: root.clone(),
        installed,
        found,
        found_rows: RefCell::new(Vec::new()),
        printers: RefCell::new(Vec::new()),
    });
    {
        let ui2 = ui.clone();
        rescan.connect_clicked(move |_| refresh_found(&ui2));
    }
    root.append(&address_group(&ui));
    root.append(&more_group());

    // Installed printers first, so the network search can skip them.
    let ui2 = ui.clone();
    in_background(load, move |snap| {
        render(&ui2, snap);
        refresh_found(&ui2);
    });

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lpstat() {
        let text = "printer HP is idle.  enabled since Mon\n\tForm mounts:\n\tDescription: HP LaserJet\n\tLocation: Office\nprinter Canon disabled since Tue -\n\tPaused\n\tDescription: Canon\n";
        let p = parse_printers(text);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].description, "HP LaserJet");
        assert_eq!(p[0].location, "Office");
        assert!(p[0].message.is_empty());
        assert!(p[1].state == State::Paused);
        assert_eq!(p[1].message, "Paused");
    }

    #[test]
    fn parses_jobs_and_names() {
        let jobs = parse_jobs("HP-12  ali  2048  Thu\nHP_2-3  ali  10  Thu\n", "HP");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].size, 2048);
        assert_eq!(unescape(r"HP\032LaserJet\032M110"), "HP LaserJet M110");
        assert_eq!(queue_name("HP LaserJet (Office)", &["HP_LaserJet_Office".into()]), "HP_LaserJet_Office_2");
        assert_eq!(txt_value(r#""txtvers=1" "rp=ipp/print" "ty=HP LaserJet""#, "ty").as_deref(), Some("HP LaserJet"));
    }
}
