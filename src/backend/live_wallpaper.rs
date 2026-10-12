//! Live wallpaper: a video that loops as the desktop background. It is a small Plasma wallpaper plugin
//! (`org.zohara.livewallpaper`, shipped in `/usr/share/plasma/wallpapers`); Settings picks the file and the options and
//! writes them with Plasma's scripting interface, which also makes Plasma use the plugin on every desktop.

use std::path::Path;
use std::process::Command;

pub const PLUGIN: &str = "org.zohara.livewallpaper";
const IMAGE_PLUGIN: &str = "org.kde.image";

pub const FILL: [(&str, u32); 3] = [("Fill the screen (crop the edges)", 0), ("Fit inside (black bars)", 1), ("Stretch", 2)];
pub const SPEED: (f64, f64) = (0.25, 2.0);

#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub path: String,
    pub fill: u32,
    pub muted: bool,
    pub speed: f64,
}

impl Default for Live {
    fn default() -> Self {
        Live { path: String::new(), fill: 0, muted: true, speed: 1.0 }
    }
}

pub fn installed() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new("/usr/share/plasma/wallpapers/org.zohara.livewallpaper").exists()
        || Path::new(&format!("{home}/.local/share/plasma/wallpapers/org.zohara.livewallpaper")).exists()
}

/// Video types the player can show (the file chooser offers these).
pub const EXTENSIONS: [&str; 7] = ["mp4", "webm", "mkv", "mov", "avi", "m4v", "gif"];

pub fn is_video(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).map(|e| EXTENSIONS.contains(&e.to_lowercase().as_str())).unwrap_or(false)
}

/// A string as a JavaScript literal for Plasma's script (the path comes from a file chooser, but may hold quotes).
fn js(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Plasma script that makes every desktop play the video with these options.
pub fn script(l: &Live) -> String {
    let speed = l.speed.clamp(SPEED.0, SPEED.1);
    format!(
        r#"desktops().forEach(function (d) {{
  d.wallpaperPlugin = "{PLUGIN}";
  d.currentConfigGroup = ["Wallpaper", "{PLUGIN}", "General"];
  d.writeConfig("VideoPath", {path});
  d.writeConfig("FillMode", {fill});
  d.writeConfig("Muted", {muted});
  d.writeConfig("Speed", {speed:.2});
  d.reloadConfig();
}});"#,
        path = js(&l.path),
        fill = l.fill.min(2),
        muted = l.muted,
    )
}

/// Plasma script that goes back to the picture wallpaper (the picture chosen before is still stored for the plugin).
pub fn stop_script() -> String {
    format!(r#"desktops().forEach(function (d) {{ d.wallpaperPlugin = "{IMAGE_PLUGIN}"; d.reloadConfig(); }});"#)
}

fn run_script(script: String) -> bool {
    Command::new("dbus-send")
        .args(["--session", "--type=method_call", "--dest=org.kde.plasmashell", "/PlasmaShell", "org.kde.PlasmaShell.evaluateScript"])
        .arg(format!("string:{script}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn apply(l: &Live) -> bool {
    run_script(script(l))
}

pub fn stop() -> bool {
    run_script(stop_script())
}

/// What the first desktop's wallpaper settings say now: `Some` when it is playing a video.
pub fn parse(appletsrc: &str) -> Option<Live> {
    let mut containment_plugin: Option<String> = None;
    let mut in_top = false;
    let mut found = false;
    let mut live = Live::default();
    let mut in_cfg = false;
    for l in appletsrc.lines().map(str::trim) {
        if l.starts_with('[') {
            in_top = l.starts_with("[Containments]") && l.matches('[').count() == 2;
            in_cfg = l.ends_with(&format!("[Wallpaper][{PLUGIN}][General]"));
            if in_top {
                containment_plugin = None;
            }
            continue;
        }
        if in_top {
            if let Some(v) = l.strip_prefix("wallpaperplugin=") {
                containment_plugin = Some(v.to_string());
                if v == PLUGIN {
                    found = true;
                }
            }
        } else if in_cfg && found {
            if let Some(v) = l.strip_prefix("VideoPath=") {
                live.path = v.to_string();
            } else if let Some(v) = l.strip_prefix("FillMode=") {
                live.fill = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("Muted=") {
                live.muted = v != "false";
            } else if let Some(v) = l.strip_prefix("Speed=") {
                live.speed = v.parse().unwrap_or(1.0);
            }
        }
    }
    let _ = containment_plugin;
    (found && !live.path.is_empty()).then_some(live)
}

/// Plasma writes its config file several seconds after a change, so the running shell is asked first and the file is
/// only the fallback (for when the shell cannot be asked).
pub fn current() -> Option<Live> {
    ask_shell().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        std::fs::read_to_string(format!("{home}/.config/plasma-org.kde.plasma.desktop-appletsrc")).ok().and_then(|t| parse(&t))
    })
}

const READ_SCRIPT: &str = r#"var d = desktops()[0];
if (d.wallpaperPlugin == "org.zohara.livewallpaper") {
  d.currentConfigGroup = ["Wallpaper", "org.zohara.livewallpaper", "General"];
  print("live|" + d.readConfig("VideoPath", "") + "|" + d.readConfig("FillMode", 0) + "|" + d.readConfig("Muted", true) + "|" + d.readConfig("Speed", 1));
} else {
  print("none");
}"#;

/// Outer `None`: the shell could not be asked. `Some(None)`: not a live wallpaper.
fn ask_shell() -> Option<Option<Live>> {
    let out = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.kde.plasmashell", "--object-path", "/PlasmaShell", "--method", "org.kde.PlasmaShell.evaluateScript", READ_SCRIPT])
        .output()
        .ok()?;
    out.status.success().then(|| parse_answer(&String::from_utf8_lossy(&out.stdout)))
}

/// `('live|/home/me/v.mp4|1|false|1.5',)` -> Live; `('none',)` -> None.
pub fn parse_answer(answer: &str) -> Option<Live> {
    let text = answer.trim().trim_start_matches('(').trim_end_matches(')').trim_end_matches(',').trim_matches('\'');
    let mut parts = text.splitn(5, '|');
    if parts.next()? != "live" {
        return None;
    }
    let path = parts.next()?.to_string();
    let fill = parts.next()?.parse().unwrap_or(0);
    let muted = parts.next()? != "false";
    let speed = parts.next()?.parse().unwrap_or(1.0);
    (!path.is_empty()).then_some(Live { path, fill, muted, speed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_sets_the_plugin_and_every_option() {
        let s = script(&Live { path: "/home/me/Videos/loop.mp4".into(), fill: 1, muted: false, speed: 0.5 });
        assert!(s.contains("d.wallpaperPlugin = \"org.zohara.livewallpaper\""));
        assert!(s.contains("writeConfig(\"VideoPath\", \"/home/me/Videos/loop.mp4\")"));
        assert!(s.contains("\"FillMode\", 1") && s.contains("\"Muted\", false") && s.contains("\"Speed\", 0.50"));
    }

    #[test]
    fn a_path_with_quotes_and_backslashes_cannot_break_out_of_the_script() {
        let s = script(&Live { path: "/tmp/a\"b\\c\nd.mp4".into(), ..Live::default() });
        assert!(s.contains(r#""/tmp/a\"b\\c\nd.mp4""#), "{s}");
        assert!(!s.contains("a\"b"), "an unescaped quote got through");
    }

    #[test]
    fn options_are_kept_in_range() {
        let s = script(&Live { speed: 99.0, fill: 77, ..Live::default() });
        assert!(s.contains("\"Speed\", 2.00") && s.contains("\"FillMode\", 2"));
    }

    #[test]
    fn stopping_goes_back_to_the_picture_plugin() {
        assert!(stop_script().contains("org.kde.image"));
    }

    #[test]
    fn video_files_are_recognised_by_extension_in_any_case() {
        assert!(is_video("/a/b.MP4") && is_video("c.webm") && is_video("d.gif"));
        assert!(!is_video("e.png") && !is_video("noextension"));
    }

    const RC: &str = "[Containments][1]\nplugin=org.kde.plasma.folder\nwallpaperplugin=org.zohara.livewallpaper\n\n[Containments][1][Wallpaper][org.zohara.livewallpaper][General]\nFillMode=1\nMuted=false\nSpeed=1.5\nVideoPath=/home/me/v.mp4\n\n[Containments][2]\nplugin=org.kde.panel\n";

    #[test]
    fn the_playing_video_is_read_from_plasmas_config() {
        assert_eq!(parse(RC), Some(Live { path: "/home/me/v.mp4".into(), fill: 1, muted: false, speed: 1.5 }));
    }

    #[test]
    fn the_shells_answer_is_read() {
        assert_eq!(
            parse_answer("('live|/home/me/v.mp4|1|false|1.5',)\n"),
            Some(Live { path: "/home/me/v.mp4".into(), fill: 1, muted: false, speed: 1.5 })
        );
        assert_eq!(parse_answer("('none',)"), None);
        assert_eq!(parse_answer("('live||0|true|1',)"), None);
    }

    #[test]
    fn a_picture_wallpaper_is_not_a_live_one() {
        assert_eq!(parse("[Containments][1]\nwallpaperplugin=org.kde.image\n"), None);
    }

    #[test]
    fn the_plugin_files_in_this_repo_match_the_names_used_here() {
        let xml = include_str!("../../data/live-wallpaper/contents/config/main.xml");
        for key in ["VideoPath", "FillMode", "Muted", "Speed"] {
            assert!(xml.contains(&format!("name=\"{key}\"")), "main.xml lacks {key}");
        }
        assert!(include_str!("../../data/live-wallpaper/metadata.json").contains(PLUGIN));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Plays a video as the real wallpaper for ten seconds, checks Plasma took it, then goes back to the picture.
    #[test]
    #[ignore]
    fn live_wallpaper_plays_on_the_real_desktop_and_the_picture_comes_back() {
        let video = "/tmp/claude-1000/dbg/test-loop.mp4";
        assert!(Path::new(video).exists(), "make the test video first");
        let before = std::fs::read_to_string(format!("{}/.config/plasma-org.kde.plasma.desktop-appletsrc", std::env::var("HOME").unwrap())).unwrap();
        assert!(apply(&Live { path: video.into(), fill: 0, muted: true, speed: 1.0 }));
        std::thread::sleep(std::time::Duration::from_secs(10));
        let now = current();
        println!("plasma says: {now:?}");
        assert_eq!(now.map(|l| l.path), Some(video.to_string()), "Plasma did not take the video");
        assert!(stop());
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert!(current().is_none(), "still a live wallpaper after stop");
        let after = std::fs::read_to_string(format!("{}/.config/plasma-org.kde.plasma.desktop-appletsrc", std::env::var("HOME").unwrap())).unwrap();
        println!("wallpaperplugin lines before: {:?}", before.lines().filter(|l| l.starts_with("wallpaperplugin=")).collect::<Vec<_>>());
        println!("wallpaperplugin lines after:  {:?}", after.lines().filter(|l| l.starts_with("wallpaperplugin=")).collect::<Vec<_>>());
    }
}
