//! Safety mode for Windows programs: the program is started inside **bubblewrap** (the technology Flatpak uses), so a
//! program that turns out to be bad can only reach its own Windows folder and its own program folder, not the
//! person's files, passwords or other programs.
//!
//! What the program sees:
//! * the system (programs, libraries, drivers, fonts) read-only, and the graphics, sound and input devices;
//! * a **empty home folder** (nothing of the real one: no Documents, no browser data, no SSH keys, no Steam login);
//! * its own Windows folder (read and write), Proton-GE (read only) and the folder the program is in (read and write,
//!   or read only when that folder is a broad one such as Downloads);
//! * the display and sound connections, but not the session's message bus, the password agents or other programs' files.
//!
//! Limits, told in the interface too: it limits damage, it is not an antivirus; the program can still use the internet
//! (unless that is switched off); anti-cheat and some copy protection will not run inside it; Game Mode does not
//! reach into it; and a program that is started some other way is not covered.

use std::path::{Path, PathBuf};

/// Everything the wrapper needs to know about the program and this computer.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub home: PathBuf,
    pub runtime_dir: PathBuf,
    /// The folder of the program file and whether it may write there.
    pub program_dir: PathBuf,
    pub program_dir_writable: bool,
    /// Windows folders the program may write to (Proton's and/or Wine's).
    pub writable: Vec<PathBuf>,
    /// Folders it may read (Proton-GE).
    pub readable: Vec<PathBuf>,
    /// Folders that must exist but stay empty (Steam's, which Proton looks for even without Steam).
    pub empty_dirs: Vec<PathBuf>,
    pub no_network: bool,
    /// Files in the real home that the display connection needs (`.Xauthority`).
    pub display_files: Vec<PathBuf>,
    /// Sockets in the runtime folder that are let through (display, sound).
    pub sockets: Vec<PathBuf>,
}

fn s(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

fn push(args: &mut Vec<String>, flag: &str, a: &str, b: &str) {
    args.push(flag.into());
    args.push(a.into());
    args.push(b.into());
}

/// The bubblewrap options for `plan` (everything before the program's own command).
pub fn args(plan: &Plan) -> Vec<String> {
    // (no --die-with-parent: the program has to go on running after the starter has finished watching it)
    let mut a: Vec<String> = vec!["--new-session".into()];
    push(&mut a, "--ro-bind", "/", "/");
    // nothing of the real home, other disks or the session's runtime folder
    // (a folder that is not there cannot be made on the read-only copy of the system, and has nothing to hide)
    a.push("--tmpfs".into());
    a.push(s(&plan.home));
    for hidden in ["/root", "/mnt", "/media", "/run", "/tmp", "/var/tmp"] {
        if Path::new(hidden).exists() {
            a.push("--tmpfs".into());
            a.push(hidden.to_string());
        }
    }
    push(&mut a, "--dev-bind", "/dev", "/dev");
    a.push("--proc".into());
    a.push("/proc".into());
    a.push("--dir".into());
    a.push(s(&plan.home));
    a.push("--dir".into());
    a.push(s(&plan.runtime_dir));
    a.push("--chmod".into());
    a.push("0700".into());
    a.push(s(&plan.runtime_dir));
    // names still resolve (the resolver's file lives in /run) and gamepads are seen (udev)
    for keep in ["/run/systemd/resolve", "/run/udev"] {
        if Path::new(keep).exists() {
            push(&mut a, "--ro-bind", keep, keep);
        }
    }
    if Path::new("/tmp/.X11-unix").exists() {
        push(&mut a, "--ro-bind", "/tmp/.X11-unix", "/tmp/.X11-unix");
    }
    for f in &plan.display_files {
        if f.exists() {
            push(&mut a, "--ro-bind", &s(f), &s(f));
        }
    }
    for sock in &plan.sockets {
        if sock.exists() {
            push(&mut a, "--bind", &s(sock), &s(sock));
        }
    }
    for d in &plan.empty_dirs {
        a.push("--dir".into());
        a.push(s(d));
    }
    for r in &plan.readable {
        if r.exists() {
            push(&mut a, "--ro-bind", &s(r), &s(r));
        }
    }
    for w in &plan.writable {
        if w.exists() {
            push(&mut a, "--bind", &s(w), &s(w));
        }
    }
    if plan.program_dir.exists() {
        let flag = if plan.program_dir_writable { "--bind" } else { "--ro-bind" };
        push(&mut a, flag, &s(&plan.program_dir), &s(&plan.program_dir));
    }
    if plan.no_network {
        a.push("--unshare-net".into());
    }
    a.push("--unshare-pid".into());
    a.push("--".into());
    a
}

/// `words` (the program's own command) started inside the sandbox.
pub fn wrap(words: Vec<String>, plan: &Plan) -> Vec<String> {
    let mut out = vec!["bwrap".to_string()];
    out.extend(args(plan));
    out.extend(words);
    out
}

/// Whether bubblewrap can be used here (it is installed and the system allows user namespaces).
pub fn available() -> bool {
    std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "true"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            home: PathBuf::from("/home/me"),
            runtime_dir: PathBuf::from("/run/user/1000"),
            program_dir: PathBuf::from("/"),
            program_dir_writable: true,
            writable: vec![PathBuf::from("/")],
            readable: vec![PathBuf::from("/usr")],
            empty_dirs: vec![PathBuf::from("/home/me/.local/share/Steam")],
            no_network: false,
            display_files: vec![],
            sockets: vec![],
        }
    }

    fn at(a: &[String], flag: &str, target: &str) -> bool {
        a.windows(3).any(|w| w[0] == flag && w[2] == target)
    }

    #[test]
    fn the_real_home_is_hidden_and_only_the_given_folders_come_back() {
        let a = args(&plan());
        assert!(a.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == "/home/me"), "home is replaced by an empty folder");
        assert!(a.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == "/run"));
        assert!(a.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == "/mnt"));
        assert!(at(&a, "--ro-bind", "/") && at(&a, "--dev-bind", "/dev"));
        assert!(at(&a, "--bind", "/"), "the writable Windows folder and program folder are bound");
        assert!(at(&a, "--ro-bind", "/usr"), "Proton-GE is read only");
        assert!(a.windows(2).any(|w| w[0] == "--dir" && w[1] == "/home/me/.local/share/Steam"));
        assert_eq!(a.last().unwrap(), "--");
        assert!(!a.contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn a_broad_program_folder_is_read_only_and_the_network_can_be_cut() {
        let mut p = plan();
        p.program_dir_writable = false;
        p.no_network = true;
        let a = args(&p);
        // the last bind of the program folder is read only
        let last = a.windows(3).rev().find(|w| w[2] == "/" && (w[0] == "--bind" || w[0] == "--ro-bind")).unwrap();
        assert_eq!(last[0], "--ro-bind");
        assert!(a.contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn the_command_follows_the_options() {
        let w = wrap(vec!["env".into(), "A=b".into(), "wine".into(), "x.exe".into()], &plan());
        assert_eq!(w[0], "bwrap");
        let sep = w.iter().position(|x| x == "--").unwrap();
        assert_eq!(&w[sep + 1..], ["env", "A=b", "wine", "x.exe"]);
    }
}
