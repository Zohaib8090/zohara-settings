//! Diagnostics: a persistent log, crash reports, and the "last run crashed"
//! marker.
//!
//! Everything goes under `$XDG_STATE_HOME/zohara` (usually
//! `~/.local/state/zohara`):
//!   settings.log            rolling log (1 MB, one previous file kept)
//!   crashes/settings-*.txt  one report per crash, with a backtrace
//!   crashed                 marker so the next launch can offer the report

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_LOG: u64 = 1024 * 1024;

pub fn state_dir() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"));
    base.join("zohara")
}

pub fn log_path() -> PathBuf {
    state_dir().join("settings.log")
}

pub fn crash_dir() -> PathBuf {
    state_dir().join("crashes")
}

fn crashed_marker() -> PathBuf {
    state_dir().join("crashed")
}

pub fn timestamp() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|t| t.format("%Y-%m-%d %H:%M:%S").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Page currently shown, recorded so crash reports say where it happened.
static CURRENT_PAGE: Mutex<String> = Mutex::new(String::new());

pub fn set_current_page(name: &str) {
    if let Ok(mut p) = CURRENT_PAGE.lock() {
        *p = name.to_string();
    }
    log::info!("opened page: {name}");
}

struct FileLogger {
    file: Mutex<Option<fs::File>>,
}

impl FileLogger {
    fn open() -> Option<fs::File> {
        let _ = fs::create_dir_all(state_dir());
        let path = log_path();
        if fs::metadata(&path).map(|m| m.len() > MAX_LOG).unwrap_or(false) {
            let _ = fs::rename(&path, path.with_extension("log.1"));
        }
        OpenOptions::new().create(true).append(true).open(path).ok()
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info || std::env::var_os("ZOHARA_DEBUG").is_some()
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:<5} [{}] {}\n", timestamp(), r.level(), r.target(), r.args());
        // stderr lands in the systemd journal for apps started from Plasma.
        eprint!("{line}");
        if let Ok(mut f) = self.file.lock() {
            if let Some(file) = f.as_mut() {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            if let Some(file) = f.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

/// Install the logger and panic hook. Call first thing in main().
pub fn init() {
    let logger: &'static FileLogger = Box::leak(Box::new(FileLogger { file: Mutex::new(FileLogger::open()) }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
    log::info!("Zohara Settings {} starting", env!("CARGO_PKG_VERSION"));

    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".into());
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let page = CURRENT_PAGE.lock().map(|p| p.clone()).unwrap_or_default();
        let bt = std::backtrace::Backtrace::force_capture();
        let report = format!(
            "Zohara Settings crash report\n\
             Version: {}\nTime: {}\nPage: {}\nError: {msg}\nAt: {loc}\nThread: {}\n\n{}\n\nBacktrace:\n{bt}\n",
            env!("CARGO_PKG_VERSION"),
            timestamp(),
            if page.is_empty() { "(none)" } else { &page },
            std::thread::current().name().unwrap_or("unnamed"),
            system_summary(),
        );
        log::error!("PANIC on page '{page}': {msg} at {loc}");
        let _ = fs::create_dir_all(crash_dir());
        let name = format!("settings-{}.txt", timestamp().replace([' ', ':'], "-"));
        let path = crash_dir().join(name);
        if fs::write(&path, &report).is_ok() {
            let _ = fs::write(crashed_marker(), path.to_string_lossy().as_bytes());
        }
        default(info);
    }));
}

/// Crash report path from the previous run, if it crashed; clears the marker.
pub fn take_previous_crash() -> Option<PathBuf> {
    let marker = crashed_marker();
    let path = fs::read_to_string(&marker).ok().map(|s| PathBuf::from(s.trim()));
    let _ = fs::remove_file(marker);
    path.filter(|p| p.exists())
}

/// Short description of the system for reports.
pub fn system_summary() -> String {
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    let os = read("/etc/os-release")
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string()))
        .unwrap_or_default();
    let kernel = read("/proc/sys/kernel/osrelease").trim().to_string();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    format!("OS: {os}\nKernel: {kernel}\nSession: {desktop} ({session})")
}

/// Run `f`, turning a panic into an error message instead of taking the app down.
pub fn guard<T>(what: &str, f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".into());
        log::error!("{what} failed: {msg}");
        // Handled here, so Settings didn't close: keep the report but don't
        // announce a crash on the next launch.
        let _ = fs::remove_file(crashed_marker());
        msg
    })
}
