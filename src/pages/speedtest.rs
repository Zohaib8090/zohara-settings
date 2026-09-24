//! Internet speed test against Cloudflare's public speed-test service
//! (speed.cloudflare.com), measured with curl: latency and jitter from
//! repeated tiny requests, download and upload from several parallel
//! transfers, like browser-based speed tests do.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;
use std::time::Instant;

const BASE: &str = "https://speed.cloudflare.com";
const STREAMS: usize = 4;
const DOWN_BYTES: u64 = 25_000_000;
const UP_BYTES: usize = 8_000_000;
const MAX_SECS: &str = "15";

fn curl(args: &[&str]) -> Option<String> {
    let o = Command::new("curl").args(["-s", "-o", "/dev/null"]).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

struct ServerInfo {
    /// Cloudflare data centre code, e.g. "KHI".
    colo: String,
    asn: String,
}

/// Cloudflare reports the serving data centre and the client's network in
/// response headers (its `/meta` endpoint no longer carries this).
fn server_info() -> Option<ServerInfo> {
    let o = Command::new("curl")
        .args(["-s", "-D", "-", "-o", "/dev/null", "--max-time", "5", &format!("{BASE}/__down?bytes=0")])
        .output()
        .ok()?;
    let headers = String::from_utf8_lossy(&o.stdout);
    let header = |name: &str| {
        headers.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    let colo = header("colo")
        .or_else(|| header("cf-ray").and_then(|r| r.rsplit('-').next().map(str::to_string)))?;
    Some(ServerInfo { colo, asn: header("asn").unwrap_or_default() })
}

/// (latency ms, jitter ms): time from TLS established to first byte, i.e. one round trip.
fn measure_latency() -> Option<(f64, f64)> {
    let url = format!("{BASE}/__down?bytes=0");
    let mut samples: Vec<f64> = (0..8)
        .filter_map(|_| {
            let out = curl(&["--max-time", "5", "-w", "%{time_appconnect} %{time_starttransfer}", &url])?;
            let mut it = out.split_whitespace().filter_map(|x| x.replace(',', ".").parse::<f64>().ok());
            let (connect, first) = (it.next()?, it.next()?);
            Some((first - connect) * 1000.0)
        })
        .filter(|ms| *ms > 0.0)
        .collect();
    if samples.len() < 3 {
        return None;
    }
    let jitter = samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (samples.len() - 1) as f64;
    samples.sort_by(|a, b| a.total_cmp(b));
    Some((samples[samples.len() / 2], jitter))
}

/// Runs `STREAMS` curl transfers at once; returns Mbit/s over the wall-clock time.
fn parallel_transfer(make_args: impl Fn() -> Vec<String> + Sync) -> Option<f64> {
    let start = Instant::now();
    let bytes: u64 = std::thread::scope(|s| {
        let handles: Vec<_> = (0..STREAMS)
            .map(|_| {
                let args = make_args();
                s.spawn(move || {
                    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    // Timeouts still report the bytes moved so far, so read the output regardless of status.
                    let o = Command::new("curl").args(["-s", "-o", "/dev/null"]).args(&refs).output().ok()?;
                    String::from_utf8_lossy(&o.stdout).trim().replace(',', ".").parse::<f64>().ok().map(|b| b as u64)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok().flatten()).sum()
    });
    let secs = start.elapsed().as_secs_f64();
    (bytes > 0 && secs > 0.0).then(|| bytes as f64 * 8.0 / secs / 1_000_000.0)
}

fn measure_download() -> Option<f64> {
    let url = format!("{BASE}/__down?bytes={DOWN_BYTES}");
    parallel_transfer(|| vec!["--max-time".into(), MAX_SECS.into(), "-w".into(), "%{size_download}".into(), url.clone()])
}

fn measure_upload() -> Option<f64> {
    // Incompressible payload so nothing along the way can shrink it.
    let path = std::env::temp_dir().join(format!("zohara-speedtest-{}.bin", std::process::id()));
    let mut data = vec![0u8; UP_BYTES];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for b in data.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    std::fs::write(&path, &data).ok()?;
    let file = format!("@{}", path.display());
    let url = format!("{BASE}/__up");
    let result = parallel_transfer(|| {
        vec![
            "--max-time".into(),
            MAX_SECS.into(),
            "-H".into(),
            "Content-Type: application/octet-stream".into(),
            "--data-binary".into(),
            file.clone(),
            "-w".into(),
            "%{size_upload}".into(),
            url.clone(),
        ]
    });
    let _ = std::fs::remove_file(&path);
    result
}

fn fmt_speed(mbps: Option<f64>) -> String {
    match mbps {
        Some(v) if v >= 100.0 => format!("{v:.0} Mbps"),
        Some(v) => format!("{v:.1} Mbps"),
        None => "Failed".into(),
    }
}

fn result_row(title: &str, icon: &str) -> (adw::ActionRow, gtk4::Label) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.add_prefix(&gtk4::Image::from_icon_name(icon));
    row.set_activatable(false);
    let value = gtk4::Label::new(Some("—"));
    value.add_css_class("title-3");
    value.add_css_class("numeric");
    row.add_suffix(&value);
    (row, value)
}

pub fn group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Speed test");
    g.set_description(Some(
        "Measures your connection using Cloudflare's speed test servers. Uses about 150 MB of data.",
    ));

    let start = gtk4::Button::with_label("Start test");
    start.add_css_class("suggested-action");
    start.set_valign(gtk4::Align::Center);
    g.set_header_suffix(Some(&start));

    let status = adw::ActionRow::new();
    status.set_title("Ready");
    status.set_activatable(false);
    let spinner = gtk4::Spinner::new();
    status.add_suffix(&spinner);
    g.add(&status);

    let (down_row, down) = result_row("Download", "go-down-symbolic");
    let (up_row, up) = result_row("Upload", "go-up-symbolic");
    let (ping_row, ping) = result_row("Ping", "network-transmit-receive-symbolic");
    let (jit_row, jitter) = result_row("Jitter", "view-continuous-symbolic");
    for r in [&down_row, &up_row, &ping_row, &jit_row] {
        g.add(r);
    }

    start.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        spinner.start();
        for l in [&down, &up, &ping, &jitter] {
            l.set_label("—");
        }
        status.set_title("Connecting…");
        status.set_subtitle("");

        let (btn, spinner, status, down, up, ping, jitter) =
            (btn.clone(), spinner.clone(), status.clone(), down.clone(), up.clone(), ping.clone(), jitter.clone());
        let finish = {
            let (btn, spinner) = (btn.clone(), spinner.clone());
            move || {
                spinner.stop();
                btn.set_sensitive(true);
                btn.set_label("Test again");
            }
        };

        in_background(
            || (server_info(), measure_latency()),
            move |(info, latency)| {
                let Some((lat, jit)) = latency else {
                    status.set_title("Couldn't reach the speed test server");
                    status.set_subtitle("Check that you're connected to the internet");
                    finish();
                    return;
                };
                ping.set_label(&format!("{lat:.0} ms"));
                jitter.set_label(&format!("{jit:.1} ms"));
                if let Some(info) = info {
                    let server = if info.asn.is_empty() {
                        format!("Cloudflare server {}", info.colo)
                    } else {
                        format!("Cloudflare server {} · Your network AS{}", info.colo, info.asn)
                    };
                    status.set_subtitle(&server);
                }
                status.set_title("Testing download speed…");

                in_background(measure_download, move |d| {
                    down.set_label(&fmt_speed(d));
                    status.set_title("Testing upload speed…");
                    in_background(measure_upload, move |u| {
                        up.set_label(&fmt_speed(u));
                        let when = glib::DateTime::now_local()
                            .ok()
                            .and_then(|t| t.format("%X").ok())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        status.set_title(&format!("Finished at {when}"));
                        finish();
                    });
                });
            },
        );
    });

    g
}
