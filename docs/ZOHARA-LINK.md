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

## Tailscale sign-in and sign-out (2026-10-14 fix)
Bug report: "Sign in" showed "No sign-in page", the browser opened, and after signing in the page still asked to sign in.
Cause found in the owner's `tailscaled` log: the first request timed out after 15 s, so the button was pressed again; a second
`tailscale up` makes Tailscale discard the first sign-in page (`register request: http 410: auth path not found`). The page
also only trusted the daemon's overlay list, not Tailscale's own state.
Now (`backend/zohara_link.rs`, `pages/zohara_link.rs`), Settings talks to `tailscale` itself:
* the state comes from `tailscale status --json` (`Running` / `NeedsLogin` / `Stopped`);
* a sign-in that is waiting is **reused** (the `AuthURL` in the status, or an already running `tailscale up`); a second one is
  never started; the page is polled for up to 25 s;
* while a sign-in is waiting the row says so and the button opens that same page again;
* **Sign out** button when signed in (asks first; `tailscale logout`), and **Turn on** when signed in but switched off;
* real errors from `tailscale up` are shown instead of a generic message.
Checked on the owner's laptop (already signed in): the state is read and a sign-in request returns "signed in" without
starting anything. NOT exercised: a real sign-in and a real sign-out (that would have signed the owner out of their own
Tailscale).

## Game streaming buttons (2026-10-15)
Zohara Link page > **Game streaming** (`pages/zohara_link.rs::streaming_group`, `backend/zohara_link.rs`):
* **Set up game streaming** installs `gst-plugins-base/good/bad/ugly`, `gst-plugin-pipewire`, `gst-plugin-va` with one password prompt
  (`pkexec pacman -S --needed --noconfirm`, no `-Sy`). Readiness is read from the plugin files in `/usr/lib/gstreamer-1.0`
  (`streaming_state_in`, tested): WebRTC + nice, pipewire, rtp, and at least one encoder (va, nvcodec, x264).
* **Allow screen sharing** runs `zohara-linkd-stream --grant-screen`: the desktop shows its own Share window once and the token is
  kept in `~/.config/zohara-link/screencast-token`. **Choose the real screen, not a virtual one**, or the video is black
  (see `zohara-link/docs/STREAMING.md`, 2026-10-16).
* The phone needs "Show the screen" and "Control the mouse" switched on for it in the same page.
Everything about the stream itself (design, protocol, status, traps) is in the `zohara-link` repo, `docs/STREAMING.md`.

## Send a file to the phone (2026-10-16)
Each paired phone's row has a **send** button (`pages/zohara_link.rs::rebuild_devices`, enabled while the phone is connected). It opens a
file chooser, then `backend/zohara_link.rs::send_file` asks the daemon (`SEND_FILE`). The phone must press **Accept** before anything is
sent; the words for each answer are in `send_file_message` (tested). The protocol is in `zohara-link/docs/FILES.md`.

Each phone's row has two buttons: **send files** (choose one or many) and **send a folder** (zipped first). Under Quick actions, the switch
**Offer copied files, folders and pictures** (off until chosen) offers whatever is copied on this computer to the connected phones; the
phone still has to press Accept. Pure parts are tested (`send_summary`, the option in `parse_status`).

