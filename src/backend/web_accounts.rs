//! Accounts for web apps. An account is a browser profile that several web apps can share, so a site is signed in once
//! and every app using that account is signed in too (Gmail, Docs and YouTube under one Google account). A web app can
//! also keep its own separate sign-ins (the default), or ask which account to use every time it opens, for a computer
//! that more than one person uses.
//!
//! Accounts are local: nothing is synced to Google or to the browser vendor.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
}

/// Which sign-ins a web app uses.
#[derive(Clone, Debug, PartialEq)]
pub enum Session {
    /// Its own profile: nothing is shared with other apps (how web apps always worked).
    Own,
    /// A shared account.
    Account(String),
    /// Ask who is using it every time it opens.
    Ask,
}

/// What is saved in `app.json`: "" (own), "ask", or "account:<id>".
pub fn parse_session(s: &str) -> Session {
    match s {
        "ask" => Session::Ask,
        _ => match s.strip_prefix("account:") {
            Some(id) if !id.is_empty() => Session::Account(id.to_string()),
            _ => Session::Own,
        },
    }
}

pub fn session_string(s: &Session) -> String {
    match s {
        Session::Own => String::new(),
        Session::Ask => "ask".into(),
        Session::Account(id) => format!("account:{id}"),
    }
}

/// An id for a new account, made from its name, that no other account has.
pub fn new_id(name: &str, taken: &[Account]) -> String {
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() { "account".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|a| a.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

pub fn load(file: &Path) -> Vec<Account> {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(file: &Path, accounts: &[Account]) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(file, serde_json::to_string_pretty(accounts).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// The letters shown on an account's picture: its first one or two initials.
pub fn initials(name: &str) -> String {
    let mut it = name.split_whitespace().filter_map(|w| w.chars().next());
    let first = it.next();
    let second = it.next();
    first.into_iter().chain(second).flat_map(|c| c.to_uppercase()).collect()
}

/// The window class Chromium gives an `--app=` window, which the launcher's StartupWMClass must match so the taskbar
/// groups the window under the app's icon. Several apps sharing one browser process cannot each pass their own `--class`
/// (the running browser handles the launch), so the class comes from the address: `chrome-<host>__<path>-Default`.
pub fn app_window_class(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (host, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    format!("chrome-{}_{}-Default", host, path.replace('/', "_"))
}

/// Where an account keeps a browser's profile. A Flatpak browser can only write under its own `~/.var/app` folder.
pub fn profile_path(data_home: &Path, home: &Path, browser: &str, flatpak: bool, account_id: &str) -> PathBuf {
    if flatpak {
        home.join(".var/app").join(browser).join("data/zohara-webapps/accounts").join(account_id).join("profile")
    } else {
        data_home.join("zohara-webapps/.accounts").join(account_id).join(browser).join("profile")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(id: &str) -> Account {
        Account { id: id.into(), name: id.into() }
    }

    #[test]
    fn session_text_round_trips_and_old_apps_stay_separate() {
        for s in [Session::Own, Session::Ask, Session::Account("work".into())] {
            assert_eq!(parse_session(&session_string(&s)), s);
        }
        assert_eq!(parse_session(""), Session::Own);
        assert_eq!(parse_session("account:"), Session::Own);
        assert_eq!(parse_session("something else"), Session::Own);
    }

    #[test]
    fn account_ids_are_unique_and_safe_as_folder_names() {
        assert_eq!(new_id("Work Google!", &[]), "work-google");
        assert_eq!(new_id("work", &[acct("work")]), "work-2");
        assert_eq!(new_id("work", &[acct("work"), acct("work-2")]), "work-3");
        assert_eq!(new_id("../..", &[]), "account");
        assert_eq!(new_id("اکاؤنٹ", &[]), "account");
    }

    #[test]
    fn accounts_survive_saving_and_a_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("zohara-acct-test-{}", std::process::id()));
        let file = dir.join("nested/accounts.json");
        assert!(load(&file).is_empty());
        let list = vec![Account { id: "home".into(), name: "Home".into() }, Account { id: "work".into(), name: "Work".into() }];
        save(&file, &list).unwrap();
        assert_eq!(load(&file), list);
        std::fs::write(&file, "not json").unwrap();
        assert!(load(&file).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn initials_use_the_first_two_words() {
        assert_eq!(initials("Zohaib Baig"), "ZB");
        assert_eq!(initials("work"), "W");
        assert_eq!(initials("  a  b  c"), "AB");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn window_class_follows_the_address() {
        assert_eq!(app_window_class("https://web.whatsapp.com"), "chrome-web.whatsapp.com__-Default");
        assert_eq!(app_window_class("https://web.whatsapp.com/"), "chrome-web.whatsapp.com__-Default");
        assert_eq!(app_window_class("https://mail.google.com/mail/u/0/?x=1#inbox"), "chrome-mail.google.com__mail_u_0_-Default");
    }

    #[test]
    fn each_browser_gets_its_own_profile_inside_an_account() {
        let (d, h) = (Path::new("/d"), Path::new("/h"));
        let a = profile_path(d, h, "brave", false, "work");
        let b = profile_path(d, h, "chromium", false, "work");
        assert_ne!(a, b);
        assert_eq!(a, PathBuf::from("/d/zohara-webapps/.accounts/work/brave/profile"));
        assert_eq!(profile_path(d, h, "com.google.Chrome", true, "work"), PathBuf::from("/h/.var/app/com.google.Chrome/data/zohara-webapps/accounts/work/profile"));
    }
}
