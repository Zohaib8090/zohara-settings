//! Reads the log of a Windows program that closed right away and says why, in plain words, and what Zohara can do about it.
//!
//! Wine and Proton write real reasons into the log (`err:module:import_dll Library MSVCP140.dll … not found`, a .NET
//! `Could not load file or assembly`, a program's own "WebView2 runtime not found"). People should not have to read that. Each
//! rule here matches such lines and gives a sentence, and, when a missing Windows part is the cause, the component to install
//! (see `game_tools::COMPONENTS`) so the notice can offer "Install it and try again".

/// A part Zohara can install, and the button text for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub component: &'static str,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    pub text: String,
    pub fix: Option<Fix>,
}

fn has_any(log: &str, words: &[&str]) -> bool {
    let low = log.to_lowercase();
    words.iter().any(|w| low.contains(&w.to_lowercase()))
}

/// A DLL Wine could not find: `err:module:import_dll Library X.dll (which is needed by …) not found`.
fn missing_dll(log: &str, dll_prefix: &str) -> bool {
    let low = log.to_lowercase();
    low.lines().any(|l| l.contains(&dll_prefix.to_lowercase()) && (l.contains("not found") || l.contains("failed") || l.contains("could not load")))
}

fn fix(component: &'static str, label: &str) -> Option<Fix> {
    Some(Fix { component, label: label.to_string() })
}

/// The first rule that matches the log (most specific first), or `None` when the cause is not known.
pub fn diagnose(log: &str) -> Option<Diagnosis> {
    // Microsoft Store (WinRT) programs: no Windows part fixes this
    if has_any(log, &["UniversalApiContract", "Windows.UI.Xaml", "Windows.ApplicationModel"]) {
        return Some(Diagnosis {
            text: "This program is built on Windows' own app platform (the Microsoft Store kind, called WinRT), which Wine and Proton do not have. That is why Microsoft's Minecraft Launcher installer cannot run here. For Minecraft, use the Java Edition with Modrinth or Prism Launcher (Gaming > Java), or Bedrock with mcpelauncher. For other programs, look for a Linux version: many apps (such as Hermes) have one.".into(),
            fix: None,
        });
    }
    // The program's installer runs a PowerShell script (Hermes' installer does); Wine has no PowerShell. Checked before the web
    // view rule: a log can hold an old web view error from before that part was installed.
    if has_any(log, &["install.ps1", "powershell.exe", "-Manifest produced no parseable", "powershell is not recognized", "pwsh.exe"]) {
        return Some(Diagnosis {
            text: "This installer works by running a PowerShell script, and Wine and Proton do not come with PowerShell, so it stops at that step. Look for a Linux version of the program instead: many apps have one (Hermes does: run `hermes desktop`).".into(),
            fix: None,
        });
    }
    if has_any(log, &["webview2", "msedgewebview2", "EdgeWebView"]) && has_any(log, &["not found", "not installed", "missing", "failed", "could not", "cannot", "unable", "error", "0x80070002", "0x80004005"]) {
        return Some(Diagnosis {
            text: "This program shows web pages inside its window using Microsoft's WebView2 runtime, which is not installed in this Windows folder yet.".into(),
            fix: fix("webview2", "Install WebView2 and try again"),
        });
    }
    if missing_dll(log, "msvcp140") || missing_dll(log, "vcruntime140") || missing_dll(log, "msvcp1") && has_any(log, &["vcruntime"]) {
        return Some(Diagnosis {
            text: "A Windows runtime library (Visual C++) that this program needs is missing from this Windows folder.".into(),
            fix: fix("vcrun2022", "Install the Visual C++ runtime and try again"),
        });
    }
    if missing_dll(log, "d3dcompiler_4") {
        return Some(Diagnosis {
            text: "The DirectX shader compiler this program needs is missing from this Windows folder.".into(),
            fix: fix("d3dcompiler_47", "Install it and try again"),
        });
    }
    if has_any(log, &["Could not load file or assembly", "mscoree.dll", "System.IO.FileNotFoundException"]) {
        return Some(Diagnosis {
            text: "This .NET program needs a Windows part that is missing from this Windows folder.".into(),
            fix: fix("dotnet48", ".NET Framework 4.8 (takes a while). Install it and try again"),
        });
    }
    if has_any(log, &["Could not connect to bus", "gamemodeauto"]) && !has_any(log, &["err:", "Unhandled Exception"]) {
        return Some(Diagnosis { text: "Game Mode could not reach the session while Safety mode was on. It is left out in Safety mode now; start the program again.".into(), fix: None });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_minecraft_installer_log_is_a_winrt_program_with_no_fix() {
        let log = "ntsync: up and running.\n[ERROR] FATAL UNHANDLED EXCEPTION: System.IO.FileNotFoundException: Could not load file or assembly 'Windows.Foundation.UniversalApiContract, Version=7.0.0.0, Culture=neutral, PublicKeyToken=null' or one of its dependencies.";
        let d = diagnose(log).unwrap();
        assert!(d.text.contains("WinRT") && d.text.contains("Minecraft") && d.text.contains("Modrinth"));
        assert!(d.fix.is_none(), "installing .NET cannot add WinRT");
    }

    #[test]
    fn a_missing_webview_offers_to_install_it() {
        let d = diagnose("Error: WebView2 Runtime not found. Please install the Microsoft Edge WebView2 Runtime.").unwrap();
        assert_eq!(d.fix.as_ref().unwrap().component, "webview2");
        assert!(d.text.contains("WebView2"));
        assert_eq!(diagnose("failed to create webview: 0x80070002 (msedgewebview2.exe)").unwrap().fix.unwrap().component, "webview2");
    }

    #[test]
    fn a_missing_visual_cpp_library_offers_the_runtime() {
        let log = "0024:err:module:import_dll Library MSVCP140.dll (which is needed by L\"C:\\\\Games\\\\x.exe\") not found\n0024:err:module:LdrInitializeThunk Importing dlls for L\"C:\\\\Games\\\\x.exe\" failed, status c0000135";
        assert_eq!(diagnose(log).unwrap().fix.unwrap().component, "vcrun2022");
        let log = "err:module:import_dll Library VCRUNTIME140_1.dll (which is needed by L\"x.exe\") not found";
        assert_eq!(diagnose(log).unwrap().fix.unwrap().component, "vcrun2022");
    }

    #[test]
    fn other_missing_parts_are_matched() {
        assert_eq!(diagnose("err:module:import_dll Library d3dcompiler_43.dll (which is needed by L\"g.exe\") not found").unwrap().fix.unwrap().component, "d3dcompiler_47");
        let d = diagnose("Unhandled Exception:\nSystem.IO.FileNotFoundException: Could not load file or assembly 'Newtonsoft.Json'").unwrap();
        assert_eq!(d.fix.unwrap().component, "dotnet48");
    }

    #[test]
    fn game_mode_noise_alone_is_explained_and_an_unknown_log_is_not() {
        assert!(diagnose("GameMode ERROR: Could not connect to bus: Failed to connect to socket").unwrap().fix.is_none());
        assert!(diagnose("fixme:ntdll:NtQuerySystemInformation stub\nall is well").is_none());
        assert!(diagnose("").is_none());
    }

    #[test]
    fn every_fix_is_a_component_the_tools_know() {
        for log in [
            "WebView2 not found",
            "err:module:import_dll Library MSVCP140.dll not found",
            "err:module:import_dll Library d3dcompiler_47.dll not found",
            "Could not load file or assembly 'X'",
        ] {
            let c = diagnose(log).unwrap().fix.unwrap().component;
            assert!(super::super::game_tools::valid_component(c), "{c}");
        }
    }

    #[test]
    fn the_hermes_installer_log_is_a_powershell_step_not_a_missing_web_view() {
        let log = "ERROR tauri_runtime_wry: Could not find the webview runtime, make sure it is installed\n\
ERROR hermes_bootstrap_lib::bootstrap: bootstrap FAILED stage=None error=install.ps1 -Manifest produced no parseable JSON payload";
        let d = diagnose(log).unwrap();
        assert!(d.text.contains("PowerShell"));
        assert!(d.fix.is_none(), "installing a web view again would not help");
    }
}
