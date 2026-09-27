# Zohara Settings on phones

Settings runs unmodified on Zohara for phones (Termux + proot on Android) —
same binary, same `arm-main` branch as the rest of that build. This file
covers what's phone-specific in this repo. For the whole phone system (how
it's built, installed, and the `zohara-phone` package that carries most of
the phone-only configuration), see
[`zohara-proot/README.md`](https://github.com/Zohaib8090/zohara/blob/arm-main/zohara-proot/README.md)
in the `zohara` repo.

## How it detects a phone

`src/backend/platform.rs`:

```rust
pub fn is_phone() -> bool {
    Path::new("/etc/zohara-proot").exists()
        || std::env::var("ZOHARA_PROOT").as_deref() == Ok("1")
}
```

`/etc/zohara-proot` is a marker file shipped by the `zohara-phone` package.
`ZOHARA_PROOT=1` is also set by the session script before that package is
installed. Nothing here talks to proot directly or otherwise senses the
environment — it's this one flag, checked wherever a page needs it.

## What's hidden

`platform::PHONE_HIDDEN_PAGES` — left out of the sidebar entirely on a phone,
each because of a real capability gap, not a design choice:

| Page | Why it can't work under proot |
|---|---|
| Bluetooth & devices | No BlueZ / Bluetooth hardware access |
| Network & internet | No NetworkManager (Android owns the network) |
| Accounts | `accounts-daemon` needs the system bus proot doesn't run |
| Time & language | `timedatectl`/`localectl` need systemd |
| Gaming | No GPU drivers / GameMode |
| Privacy & security | Firewall needs the kernel; camera/mic monitoring needs real device access |
| Display | Termux:X11 owns the actual display, not Zohara |
| Sound | No PipeWire devices — sound goes out through Termux's PulseAudio instead |
| Power & battery | No UPower/PowerDevil — Android manages power |
| Mouse / Touchpad | No `libinput` devices to configure |
| Printers | No CUPS |
| Zohara Link | Pairs a phone with a computer — this *is* the phone |
| Troubleshoot | Its checks are all against systemd services that don't exist here |

Two individual groups are hidden the same way, inside otherwise-visible pages:
- **Storage → System restore points**: needs Btrfs snapshots and a boot menu.
- **Accessibility → Voice typing**: needs `/dev/uinput` to type into other
  apps, which proot can't reach.

`main.rs`'s page loop skips any page in that list. Since hidden pages leave
gaps in the sidebar's row indices, `pages::goto()` (used by "Open Troubleshoot"
buttons, the crash dialog, etc.) looks up rows by their stored page index
rather than assuming index == position — a `goto("Printers")` on a phone
build simply does nothing, since no row exists for it.

## What's adjusted, not hidden

- The sidebar is narrower on a phone (`200px` vs `260px`) — a phone screen is
  narrow to begin with.
- Everything else (scale, full-screen windows, the finger-sized panel,
  turning off effects/compositing/indexing) is handled outside this repo, by
  the `zohara-phone` package's session script and its XDG config overrides —
  see `zohara-proot/README.md`. Settings itself carries no phone-specific
  theming.

## A test that keeps this honest

`platform::tests::hidden_pages_are_real_page_labels` reads `main.rs`'s own
source and asserts every string in `PHONE_HIDDEN_PAGES` matches a real
`PageDef { label: "..." }`. A typo here would otherwise silently leave a
broken page visible on phones instead of failing loudly.

## Not yet verified

Nothing in this file has been checked on a real phone. The phone build
compiles and this crate's tests pass on real aarch64 hardware in CI, but
whether the hidden-page list is complete — whether some *visible* page still
reaches for something proot doesn't have — is unverified until someone
actually clicks through Settings on a booted phone.
