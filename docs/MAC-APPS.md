# Mac and iPhone app checker (2026-10-21)

Settings > Apps > **Mac and iPhone apps**, and `zohara-settings --check-mac-app PATH` on the command line. Step 1 of the plan in `zohara/docs/RUN-ANYTHING-MACOS-IOS-2026-10-19.md`.
It reads what an app says about itself and answers in plain words. It never runs the app and never opens Apple's code.

| Input | What is read |
|---|---|
| `.app` folder | `Contents/Info.plist` (XML or binary), then the program named by `CFBundleExecutable` in `Contents/MacOS` |
| `.ipa` | `Payload/*.app/Info.plist` and the program's header, through `unzip -p` (first 8 MiB only) |
| `.dmg` | not opened: "open it first, then check the .app inside" |
| a Mach-O file (`.dylib`, a program) | the file itself, thin or "fat" |

From the Mach-O load commands: the CPU slices (`x86_64`, `arm64`, 32-bit Intel), the platform (macOS / iOS), the Apple libraries it asks for, and App Store encryption (`cryptid`).

**Answers:** *might run* (Intel build, only classic Mac libraries: worth trying under Darling), *will most likely not run* (uses Metal, SwiftUI, WebKit and the like, or is written in Swift), *cannot run* (iPhone/iPad app, or built only for Apple silicon), *cannot be checked* (disk image, data folder, unreadable file).

**Honest limits:** the library lists (`CLASSIC`, `HARD` in `backend/mac_check.rs`) come from general knowledge of what Darling covers, **not from a test**: Darling is not installed or measured yet, and every "might run" says so. iPhone apps are said to be impossible; if an app is App Store encrypted the text says Zohara will not remove that. The `.dmg` case does no unpacking yet.

Tests: 10 unit tests with Mach-O files built in the test (thin, fat, truncated, Java class file), XML and binary plists, app folders and a data folder shaped like the owner's `Cities.app`. Not tested: a real Mac app (none on the laptop), the page's two file choosers by clicking (the command line was run on a sample).
