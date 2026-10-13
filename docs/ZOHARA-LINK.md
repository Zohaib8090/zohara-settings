# Settings > Zohara Link (the page that controls the phone companion)

The phone companion (`zohara-link` repo: the daemon `zohara-linkd` on the computer and an Android app) is controlled from this page. The
page does nothing by itself: it asks the daemon over its local socket and shows what the daemon says. Files: `src/pages/zohara_link.rs`
(the page), `src/backend/zohara_link.rs` (the socket client, parsing, the address list).

## What the page shows (all of it only while the daemon is running)

* **Status**: "Zohara Link is on / turned off / not set up", with a button: **Set up Zohara Link** (installs the package with an
  administrator prompt: `pkexec pacman -S --needed zohara-linkd`, uses the package lists already on the computer, never a bare `-Sy`) or
  **Turn on** (`systemctl --user enable --now zohara-linkd.service`).
* **This computer's code**: the short form (`ab12 cd34 ef56 7890`) of the SHA-256 of the daemon's TLS certificate. The phone shows the
  same code while pairing; if they differ, someone is in the middle.
* **This computer's address**: each real IPv4 address with its type and the port (`192.168.1.20 (Wi-Fi) · port 42424`) and a **Copy**
  button. For phones that cannot find the computer by themselves. Virtual interfaces (Docker, virbr, veth, ...) are left out; Wi-Fi is
  listed first. Source: `ip -4 -o addr show scope global` (`parse_addresses`).
* **Pair a phone**: opens pairing for 5 minutes (`OPEN_PAIRING`); it closes after one phone pairs. While a phone waits, the **PIN** is shown
  here (`pending_pairings` in `GET_STATUS`) and in a desktop notification. The phone types it; it is never sent over the network.
* **Your phones**: each paired phone, connected or not, with a switch **Control the mouse** (`SET_PERMISSION`, off by default; it also
  covers locking the screen from the phone) and an **Unpair** button (confirmation; the phone loses access at once).
* **Quick actions**: send the clipboard to the phone.

The page asks again every two seconds while it exists and rebuilds the phone list only when it changed, so a switch is never replaced
under a finger.

## Away from home and the screen (2026-10-13, alpha)

* **Away from home** row (under the computer's address): lists this computer's Tailscale / ZeroTier / NetBird / WireGuard addresses
  (`GET_REMOTE`). If Tailscale is missing the button **Set up Tailscale** installs it (password prompt), starts `tailscaled` and lets this
  user control it; if it is installed but signed out the button **Sign in** (`REMOTE_LOGIN`) opens the sign-in page in the browser.
  Paired phones learn the addresses by themselves. Not tested with a real Tailscale account.
* **Show the screen** switch next to "Control the mouse" for every phone (`SET_PERMISSION` with `screen`). Off by default.

## The daemon's local socket (what this page uses)

`/run/user/UID/zohara.sock` (mode 600; override with `ZOHARA_LINK_SOCKET`), one JSON line per command, one JSON line back; events
(`PAIR_REQUEST`, `DEVICE_PAIRED`, ...) are broadcast on the same connection. Commands: `GET_STATUS` (paired devices without token hashes,
connected ids, `pairing_open`, `pending_pairings`), `GET_IDENTITY` (name, port, fingerprint), `OPEN_PAIRING`, `CLOSE_PAIRING`,
`SET_PERMISSION`, `UNPAIR`, `SEND_CLIPBOARD`, `LOCK_SCREEN`. The security model is in `zohara-link/docs/SECURITY.md`.

## Tests and proof

* Unit tests in `backend/zohara_link.rs`: status parsing, address parsing (virtual networks skipped, Wi-Fi first), PIN grouping, a fake
  daemon socket that sends event lines before the answer, and "no daemon means nothing, not a crash".
* Compile-checked in a container with libadwaita 1.5 by lowering the feature to `v1_5` in a scratch copy (see `GOTCHA.md` in the `zohara` repo).
* Run against the real daemon in Xvfb and screenshotted through a whole pairing (PIN shown, phone listed as connected, mouse switch off).
* **Not tested:** the Set up button (needs `zohara-linkd` on the update channel and a password prompt), the page on a real Plasma desktop.
