//! System health checks, shared by the Troubleshoot page and the background
//! `zohara-settings --health-check` run (a systemd user timer).
//!
//! Each check is independent and read-only; a check that can't run is
//! simply skipped, so one missing tool never hides the other results.

use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Debug)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Fix {
    RestartSystemUnit(String),
    RestartUserUnit(String),
    RemovePacmanLock,
    OpenPage(String),
    Reboot,
    EnableTimeSync,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Issue {
    /// Stable key used to avoid notifying about the same problem twice.
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub fix: Option<Fix>,
    /// Command whose output helps explain the issue (shown under "Details").
    pub log_cmd: Option<Vec<String>>,
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).to_string())
}

pub fn run_to_string(cmd: &[String]) -> String {
    let Some((bin, args)) = cmd.split_first() else { return String::new() };
    Command::new(bin)
        .args(args)
        .output()
        .map(|o| {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        })
        .unwrap_or_else(|e| e.to_string())
}

fn failed_units(user: bool) -> Vec<Issue> {
    let mut args = vec!["--failed", "--no-legend", "--plain", "--no-pager"];
    if user {
        args.insert(0, "--user");
    }
    let Some(out) = run("systemctl", &args) else { return Vec::new() };
    out.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|u| !u.is_empty())
        .map(|unit| {
            let desc = run("systemctl", &[if user { "--user" } else { "--system" }, "show", "-p", "Description", "--value", unit])
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| unit.to_string());
            let mut log = vec!["journalctl".to_string()];
            if user {
                log.push("--user".into());
            }
            log.extend(["-u", unit, "-b", "-n", "40", "--no-pager"].map(String::from));
            Issue {
                id: format!("unit:{}:{unit}", if user { "user" } else { "system" }),
                severity: Severity::Warning,
                title: format!("{desc} stopped working"),
                detail: format!("The background service “{unit}” failed. Restarting it often fixes this."),
                fix: Some(if user { Fix::RestartUserUnit(unit.into()) } else { Fix::RestartSystemUnit(unit.into()) }),
                log_cmd: Some(log),
            }
        })
        .collect()
}

fn disk_space() -> Vec<Issue> {
    let Some(out) = run("df", &["-P", "/", "/home"]) else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    out.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            let pct: u32 = c.get(4)?.trim_end_matches('%').parse().ok()?;
            let mount = c.get(5)?.to_string();
            let avail_kb: u64 = c.get(3)?.parse().ok()?;
            if !seen.insert(c.first()?.to_string()) || pct < 90 {
                return None;
            }
            Some(Issue {
                id: format!("disk:{mount}"),
                severity: if pct >= 97 { Severity::Critical } else { Severity::Warning },
                title: if mount == "/" { "Your system drive is almost full".into() } else { format!("{mount} is almost full") },
                detail: format!(
                    "{pct}% used, {:.1} GB free. Updates and apps can fail when the drive fills up.",
                    avail_kb as f64 / 1_048_576.0
                ),
                fix: Some(Fix::OpenPage("Storage".into())),
                log_cmd: None,
            })
        })
        .collect()
}

fn pending_reboot() -> Option<Issue> {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?.trim().to_string();
    // After a kernel update Arch removes the running kernel's modules, so new
    // hardware (USB drives, Wi-Fi dongles) can't be loaded until a restart.
    (!std::path::Path::new(&format!("/usr/lib/modules/{kernel}")).exists()).then(|| Issue {
        id: format!("reboot:{kernel}"),
        severity: Severity::Warning,
        title: "Restart to finish updating".into(),
        detail: "The system was updated. Until you restart, newly plugged-in devices may not work.".into(),
        fix: Some(Fix::Reboot),
        log_cmd: None,
    })
}

fn pacman_lock() -> Option<Issue> {
    let lock = std::path::Path::new("/var/lib/pacman/db.lck");
    if !lock.exists() {
        return None;
    }
    let busy = Command::new("pgrep").args(["-x", "pacman"]).status().map(|s| s.success()).unwrap_or(true);
    (!busy).then(|| Issue {
        id: "pacman-lock".into(),
        severity: Severity::Warning,
        title: "Software updates are blocked".into(),
        detail: "An earlier install or update was interrupted and left a lock behind, so no app can be installed or updated.".into(),
        fix: Some(Fix::RemovePacmanLock),
        log_cmd: Some(vec!["tail".into(), "-n".into(), "30".into(), "/var/log/pacman.log".into()]),
    })
}

fn crashes() -> Vec<Issue> {
    let Some(out) = run("coredumpctl", &["list", "--since=-24h", "--no-legend", "--no-pager", "-q"]) else { return Vec::new() };
    let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
    for line in out.lines() {
        // The executable is the last column.
        if let Some(exe) = line.split_whitespace().last() {
            if exe.starts_with('/') {
                *counts.entry(exe.to_string()).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .map(|(exe, n)| {
            let name = exe.rsplit('/').next().unwrap_or(&exe).to_string();
            Issue {
                id: format!("crash:{name}:{n}"),
                severity: if n >= 3 { Severity::Warning } else { Severity::Info },
                title: format!("{name} crashed {}", if n == 1 { "once today".into() } else { format!("{n} times today") }),
                detail: "If this keeps happening, include the details in a problem report.".into(),
                fix: None,
                log_cmd: Some(vec!["coredumpctl".into(), "info".into(), "--no-pager".into(), exe.clone()]),
            }
        })
        .collect()
}

fn memory() -> Option<Issue> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let get = |k: &str| {
        info.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    let (total, avail) = (get("MemTotal:")?, get("MemAvailable:")?);
    (avail * 100 / total.max(1) < 5).then(|| Issue {
        id: "memory".into(),
        severity: Severity::Warning,
        title: "Memory is almost full".into(),
        detail: format!("Only {} MB free. Close apps you aren't using to keep things responsive.", avail / 1024),
        fix: None,
        log_cmd: Some(vec!["sh".into(), "-c".into(), "ps -eo rss,comm --sort=-rss | head -n 12".into()]),
    })
}

fn time_sync() -> Option<Issue> {
    let show = |p: &str| run("timedatectl", &["show", "-p", p, "--value"]).map(|s| s.trim().to_string());
    if show("CanNTP")? != "yes" {
        return None;
    }
    let ntp = show("NTP")?;
    let synced = show("NTPSynchronized")?;
    (ntp != "yes").then(|| Issue {
        id: "time-sync".into(),
        severity: Severity::Info,
        title: "Automatic time is off".into(),
        detail: "A wrong clock breaks secure websites and updates.".into(),
        fix: Some(Fix::EnableTimeSync),
        log_cmd: None,
    })
    .or_else(|| {
        (synced != "yes" && connectivity_ok()).then(|| Issue {
            id: "time-unsynced".into(),
            severity: Severity::Info,
            title: "The clock hasn't synchronized yet".into(),
            detail: "Time servers couldn't be reached. This usually fixes itself once you're online.".into(),
            fix: None,
            log_cmd: Some(vec!["timedatectl".into(), "timesync-status".into()]),
        })
    })
}

fn connectivity_ok() -> bool {
    run("nmcli", &["networking", "connectivity"]).map(|s| s.trim() == "full").unwrap_or(true)
}

fn settings_crash() -> Option<Issue> {
    let dir = super::diag::crash_dir();
    let newest = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter(|e| e.metadata().and_then(|m| m.modified()).map(|t| t.elapsed().map(|d| d.as_secs() < 7 * 86400).unwrap_or(false)).unwrap_or(false))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?;
    let path = newest.path();
    Some(Issue {
        id: format!("settings-crash:{}", path.file_name()?.to_string_lossy()),
        severity: Severity::Info,
        title: "Settings closed unexpectedly recently".into(),
        detail: "A crash report was saved. Sending it helps get this fixed.".into(),
        fix: None,
        log_cmd: Some(vec!["cat".into(), path.to_string_lossy().to_string()]),
    })
}

/// All checks. Slow-ish (spawns processes); call off the UI thread.
pub fn check_all() -> Vec<Issue> {
    let mut v = Vec::new();
    v.extend(failed_units(false));
    v.extend(failed_units(true));
    v.extend(disk_space());
    v.extend(pending_reboot());
    v.extend(pacman_lock());
    v.extend(memory());
    v.extend(crashes());
    v.extend(time_sync());
    v.extend(settings_crash());
    v.sort_by_key(|i| i.severity);
    v
}

/// Carries out a fix. Returns a message for the user on failure.
pub fn apply(fix: &Fix) -> Result<(), String> {
    let ok = |c: &mut Command| c.status().map(|s| s.success()).unwrap_or(false);
    let done = match fix {
        Fix::RestartSystemUnit(u) => ok(Command::new("pkexec").args(["systemctl", "restart", u])),
        Fix::RestartUserUnit(u) => ok(Command::new("systemctl").args(["--user", "restart", u])),
        Fix::RemovePacmanLock => ok(Command::new("pkexec").args([
            "sh",
            "-c",
            "pgrep -x pacman >/dev/null || rm -f /var/lib/pacman/db.lck",
        ])),
        Fix::EnableTimeSync => ok(Command::new("timedatectl").args(["set-ntp", "true"])),
        Fix::Reboot => ok(Command::new("systemctl").arg("reboot")),
        Fix::OpenPage(_) => true,
    };
    if done {
        log::info!("applied fix {fix:?}");
        Ok(())
    } else {
        log::warn!("fix {fix:?} failed or was cancelled");
        Err("It didn't work, or administrator approval was not given.".into())
    }
}

/// Plain-text diagnostic report for sharing.
pub fn report(issues: &[Issue]) -> String {
    let mut s = format!(
        "Zohara OS problem report\nCreated: {}\nSettings: {}\n{}\n\n",
        super::diag::timestamp(),
        env!("CARGO_PKG_VERSION"),
        super::diag::system_summary()
    );
    if issues.is_empty() {
        s.push_str("No problems found.\n");
    }
    for i in issues {
        s.push_str(&format!("== [{:?}] {}\n{}\n", i.severity, i.title, i.detail));
        if let Some(cmd) = &i.log_cmd {
            let out = run_to_string(cmd);
            let tail: Vec<&str> = out.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
            s.push_str(&format!("-- {}\n{}\n", cmd.join(" "), tail.join("\n")));
        }
        s.push('\n');
    }
    s.push_str("== Recent errors this boot (journalctl -b -p err)\n");
    s.push_str(&run_to_string(&["journalctl", "-b", "-p", "err", "-n", "60", "--no-pager"].map(String::from)));
    s.push_str("\n== Settings log (last 80 lines)\n");
    let log = std::fs::read_to_string(super::diag::log_path()).unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    s.push_str(&lines[lines.len().saturating_sub(80)..].join("\n"));
    s.push('\n');
    s
}

// ── Background check (systemd user timer) ──────────────────────────────────

fn notified_path() -> std::path::PathBuf {
    super::diag::state_dir().join("health-notified.json")
}

/// Notifies about new Warning/Critical issues once each. Returns exit code.
pub fn background_check() -> i32 {
    let issues = check_all();
    let important: Vec<&Issue> = issues.iter().filter(|i| i.severity <= Severity::Warning).collect();
    let seen: Vec<String> = std::fs::read_to_string(notified_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let new: Vec<&&Issue> = important.iter().filter(|i| !seen.contains(&i.id)).collect();
    // Remember what's current, so a fixed problem that comes back is reported again.
    let _ = std::fs::create_dir_all(super::diag::state_dir());
    let _ = std::fs::write(notified_path(), serde_json::to_string(&important.iter().map(|i| &i.id).collect::<Vec<_>>()).unwrap_or_default());
    log::info!("health check: {} issues, {} new to notify", issues.len(), new.len());
    if new.is_empty() {
        return 0;
    }
    let critical = new.iter().any(|i| i.severity == Severity::Critical);
    let title = if new.len() == 1 { new[0].title.clone() } else { format!("{} problems need your attention", new.len()) };
    let body = if new.len() == 1 { new[0].detail.clone() } else { new.iter().map(|i| format!("• {}", i.title)).collect::<Vec<_>>().join("\n") };
    // --wait keeps the process until the notification is closed, so the action can be handled.
    let out = Command::new("notify-send")
        .args([
            "--app-name=Zohara Settings",
            "--icon=dialog-warning",
            &format!("--urgency={}", if critical { "critical" } else { "normal" }),
            "--action=open=Troubleshoot",
            "--wait",
            &title,
            &body,
        ])
        .output();
    if let Ok(o) = out {
        if String::from_utf8_lossy(&o.stdout).trim() == "open" {
            let exe = std::env::current_exe().unwrap_or_else(|_| "zohara-settings".into());
            let _ = Command::new(exe).args(["--page", "Troubleshoot"]).spawn();
        }
    }
    0
}
