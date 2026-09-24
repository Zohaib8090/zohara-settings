//! Troubleshoot: runs the system health checks, explains each problem in
//! plain words with a one-click fix where there is one, shows technical
//! details on demand, and builds a shareable problem report.

use crate::backend::diag;
use crate::backend::health::{self, Fix, Issue, Severity};
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

const TIMER: &str = "zohara-settings-health.timer";
const ISSUES_URL: &str = "https://github.com/Zohaib8090/zohara/issues/new";

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

fn fix_label(f: &Fix) -> String {
    match f {
        Fix::RestartSystemUnit(_) | Fix::RestartUserUnit(_) => "Restart service".into(),
        Fix::RemovePacmanLock => "Unblock".into(),
        Fix::OpenPage(p) => format!("Open {p}"),
        Fix::Reboot => "Restart now".into(),
        Fix::EnableTimeSync => "Turn on".into(),
    }
}

fn severity_icon(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "dialog-error-symbolic",
        Severity::Warning => "dialog-warning-symbolic",
        Severity::Info => "dialog-information-symbolic",
    }
}

fn details_view(text: &str) -> gtk4::ScrolledWindow {
    let buf = gtk4::TextBuffer::new(None);
    buf.set_text(text);
    let tv = gtk4::TextView::with_buffer(&buf);
    tv.set_editable(false);
    tv.set_monospace(true);
    tv.set_wrap_mode(gtk4::WrapMode::WordChar);
    tv.set_left_margin(8);
    tv.set_right_margin(8);
    tv.set_top_margin(8);
    tv.set_bottom_margin(8);
    gtk4::ScrolledWindow::builder().child(&tv).min_content_height(160).max_content_height(320).propagate_natural_height(true).build()
}

fn issue_row(issue: &Issue, page: &gtk4::Box, recheck: std::rc::Rc<dyn Fn()>) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(&glib::markup_escape_text(&issue.title));
    row.set_subtitle(&glib::markup_escape_text(&issue.detail));
    let icon = gtk4::Image::from_icon_name(severity_icon(issue.severity));
    match issue.severity {
        Severity::Critical => icon.add_css_class("error"),
        Severity::Warning => icon.add_css_class("warning"),
        Severity::Info => icon.add_css_class("dim-label"),
    }
    row.add_prefix(&icon);

    if let Some(fix) = issue.fix.clone() {
        let btn = gtk4::Button::with_label(&fix_label(&fix));
        btn.set_valign(gtk4::Align::Center);
        if issue.severity <= Severity::Warning {
            btn.add_css_class("suggested-action");
        }
        let (page2, recheck2) = (page.clone(), recheck.clone());
        btn.connect_clicked(move |b| {
            if let Fix::OpenPage(p) = &fix {
                super::goto(&page2, p);
                return;
            }
            let run = {
                let (fix, page3, recheck3, b2) = (fix.clone(), page2.clone(), recheck2.clone(), b.clone());
                move || {
                    b2.set_sensitive(false);
                    let (page4, recheck4, b3, fix2) = (page3.clone(), recheck3.clone(), b2.clone(), fix.clone());
                    in_background(
                        move || health::apply(&fix2),
                        move |res| {
                            b3.set_sensitive(true);
                            match res {
                                Ok(()) => recheck4(),
                                Err(e) => message(&page4, "Not fixed", &e),
                            }
                        },
                    );
                }
            };
            if matches!(fix, Fix::Reboot) {
                let d = adw::AlertDialog::new(Some("Restart now?"), Some("Save your work first. Open apps will close."));
                d.add_responses(&[("cancel", "Cancel"), ("restart", "Restart")]);
                d.set_response_appearance("restart", adw::ResponseAppearance::Destructive);
                d.connect_response(None, move |_, r| {
                    if r == "restart" {
                        run();
                    }
                });
                d.present(Some(&page2));
            } else {
                run();
            }
        });
        row.add_suffix(&btn);
    }

    if let Some(cmd) = issue.log_cmd.clone() {
        let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let loaded = std::rc::Rc::new(std::cell::Cell::new(false));
        row.add_row(&holder);
        row.connect_expanded_notify(move |r| {
            if !r.is_expanded() || loaded.get() {
                return;
            }
            loaded.set(true);
            let (holder, cmd) = (holder.clone(), cmd.clone());
            in_background(
                move || health::run_to_string(&cmd),
                move |out| {
                    let text = if out.trim().is_empty() { "No further details were recorded.".to_string() } else { out };
                    holder.append(&details_view(&text));
                },
            );
        });
    } else {
        row.set_enable_expansion(false);
    }
    row
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn report_group(page: &gtk4::Box, last: std::rc::Rc<std::cell::RefCell<Vec<Issue>>>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Report a problem");
    g.set_description(Some(
        "A report lists the problems above, recent system errors and Settings' log. It doesn't include your files or passwords, but review it before sharing.",
    ));

    let save = adw::ActionRow::new();
    save.set_title("Save a problem report");
    save.add_prefix(&gtk4::Image::from_icon_name("document-save-symbolic"));
    let save_btn = gtk4::Button::with_label("Save…");
    save_btn.set_valign(gtk4::Align::Center);
    {
        let (page, last) = (page.clone(), last.clone());
        save_btn.connect_clicked(move |b| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Save problem report");
            dialog.set_initial_name(Some(&format!("zohara-report-{}.txt", diag::timestamp().replace([' ', ':'], "-"))));
            let (page2, issues) = (page.clone(), last.borrow().clone());
            dialog.save(b.root().and_downcast_ref::<gtk4::Window>(), None::<&gtk4::gio::Cancellable>, move |res| {
                let Some(path) = res.ok().and_then(|f| f.path()) else { return };
                let page3 = page2.clone();
                in_background(
                    move || std::fs::write(&path, health::report(&issues)).map(|_| path),
                    move |r| match r {
                        Ok(p) => message(&page3, "Report saved", &format!("Saved to {}", p.display())),
                        Err(e) => message(&page3, "Report not saved", &e.to_string()),
                    },
                );
            });
        });
    }
    save.add_suffix(&save_btn);
    g.add(&save);

    let copy = adw::ActionRow::new();
    copy.set_title("Copy the report");
    copy.add_prefix(&gtk4::Image::from_icon_name("edit-copy-symbolic"));
    let copy_btn = gtk4::Button::with_label("Copy");
    copy_btn.set_valign(gtk4::Align::Center);
    {
        let last = last.clone();
        copy_btn.connect_clicked(move |b| {
            let issues = last.borrow().clone();
            let b2 = b.clone();
            b.set_sensitive(false);
            in_background(
                move || health::report(&issues),
                move |text| {
                    b2.clipboard().set_text(&text);
                    b2.set_sensitive(true);
                    b2.set_label("Copied");
                },
            );
        });
    }
    copy.add_suffix(&copy_btn);
    g.add(&copy);

    let gh = adw::ActionRow::new();
    gh.set_title("Report it to the Zohara team");
    gh.set_subtitle("Opens a new issue on GitHub. Attach the saved report to it");
    gh.add_prefix(&gtk4::Image::from_icon_name("mail-send-symbolic"));
    gh.add_suffix(&gtk4::Image::from_icon_name("adw-external-link-symbolic"));
    gh.set_activatable(true);
    {
        let last = last.clone();
        gh.connect_activated(move |_| {
            let issues = last.borrow();
            let title = issues.first().map(|i| i.title.clone()).unwrap_or_else(|| "Problem report".into());
            let mut body = format!("**What happened?**\n\n\n**System**\n```\n{}\nSettings {}\n```\n\n**Problems found by Troubleshoot**\n", diag::system_summary(), env!("CARGO_PKG_VERSION"));
            for i in issues.iter().take(10) {
                body.push_str(&format!("- [{:?}] {}\n", i.severity, i.title));
            }
            body.push_str("\n_Please attach the report saved from Settings > Troubleshoot._\n");
            let url = format!("{ISSUES_URL}?title={}&body={}", url_encode(&title), url_encode(&body));
            let _ = Command::new("xdg-open").arg(url).spawn();
        });
    }
    g.add(&gh);

    let logs = adw::ActionRow::new();
    logs.set_title("Open the log folder");
    logs.set_subtitle(&diag::state_dir().to_string_lossy());
    logs.add_prefix(&gtk4::Image::from_icon_name("folder-open-symbolic"));
    logs.set_activatable(true);
    logs.connect_activated(|_| {
        let _ = std::fs::create_dir_all(diag::state_dir());
        let _ = Command::new("xdg-open").arg(diag::state_dir()).spawn();
    });
    g.add(&logs);
    g
}

fn background_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    let sw = adw::SwitchRow::new();
    sw.set_title("Check for problems in the background");
    sw.set_subtitle("Get a notification when something on your system stops working");
    sw.add_prefix(&gtk4::Image::from_icon_name("preferences-system-notifications-symbolic"));
    let state = Command::new("systemctl")
        .args(["--user", "is-enabled", TIMER])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if state.is_empty() || state == "not-found" {
        sw.set_sensitive(false);
        sw.set_subtitle("The background check isn't installed on this system");
    }
    sw.set_active(state == "enabled");
    sw.connect_active_notify(|r| {
        let on = r.is_active();
        std::thread::spawn(move || {
            let _ = Command::new("systemctl").args(["--user", if on { "enable" } else { "disable" }, "--now", TIMER]).status();
        });
    });
    g.add(&sw);
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
            .label("Troubleshoot")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let problems = adw::PreferencesGroup::new();
    problems.set_title("System health");
    let recheck_btn = gtk4::Button::with_label("Check again");
    recheck_btn.set_valign(gtk4::Align::Center);
    problems.set_header_suffix(Some(&recheck_btn));
    root.append(&problems);

    let last: std::rc::Rc<std::cell::RefCell<Vec<Issue>>> = Default::default();
    let rows: std::rc::Rc<std::cell::RefCell<Vec<gtk4::Widget>>> = Default::default();
    let cell: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>> = Default::default();
    let check: std::rc::Rc<dyn Fn()> = {
        let (problems, rows, last, root, cell, btn) = (problems.clone(), rows.clone(), last.clone(), root.clone(), cell.clone(), recheck_btn.clone());
        std::rc::Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                problems.remove(&r);
            }
            btn.set_sensitive(false);
            let busy = adw::ActionRow::new();
            busy.set_title("Checking your system…");
            let spinner = gtk4::Spinner::new();
            spinner.start();
            busy.add_suffix(&spinner);
            problems.add(&busy);
            rows.borrow_mut().push(busy.clone().upcast());

            let (problems, rows, last, root, again, btn) =
                (problems.clone(), rows.clone(), last.clone(), root.clone(), cell.borrow().clone().expect("set"), btn.clone());
            in_background(health::check_all, move |issues| {
                for r in rows.borrow_mut().drain(..) {
                    problems.remove(&r);
                }
                btn.set_sensitive(true);
                let serious = issues.iter().filter(|i| i.severity <= Severity::Warning).count();
                let summary = adw::ActionRow::new();
                if issues.is_empty() {
                    summary.set_title("No problems found");
                    summary.set_subtitle("Everything is working as expected");
                    summary.add_prefix(&gtk4::Image::from_icon_name("emblem-ok-symbolic"));
                } else {
                    summary.set_title(&if serious == 0 {
                        "Nothing needs fixing".to_string()
                    } else if serious == 1 {
                        "1 problem needs your attention".to_string()
                    } else {
                        format!("{serious} problems need your attention")
                    });
                    summary.set_subtitle(&format!("Checked {}", diag::timestamp()));
                }
                problems.add(&summary);
                rows.borrow_mut().push(summary.upcast());
                for i in &issues {
                    let r = issue_row(i, &root, again.clone());
                    problems.add(&r);
                    rows.borrow_mut().push(r.upcast());
                }
                *last.borrow_mut() = issues;
            });
        })
    };
    *cell.borrow_mut() = Some(check.clone());
    {
        let check = check.clone();
        recheck_btn.connect_clicked(move |_| check());
    }
    check();

    root.append(&background_group());
    root.append(&report_group(&root, last));

    scroll.set_child(Some(&root));
    scroll.upcast()
}
