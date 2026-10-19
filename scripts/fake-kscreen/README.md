# Fake `kscreen-doctor` for testing the Display page

A stand-in with **two screens that can do everything** (brightness, VRR, HDR, wide gamut, colour options, overscan). It keeps its state in
`state.json` next to it and logs every change command in `calls.log`; it never touches a real screen.

Use it only in the VM (or any place where opening a Settings window is fine):

```
PATH=/path/to/scripts/fake-kscreen:$PATH zohara-settings --page Display
```

Close any running Settings first (it is a single-instance app, so a second start would just show the real one). Delete `state.json` to start over.
The `-o` text format (`Vrr: Automatic`, `HDR: disabled` with an indented `SDR brightness: 300`) was read from the strings of the real `kscreen-doctor` 6.7
and from the laptop panel; the exact text of a **capable** monitor was never seen on real hardware.
