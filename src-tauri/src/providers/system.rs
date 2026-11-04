use crate::commands::{ResultKind, SearchResult};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use std::process::Command;

/// (id, title, subtitle). Settings pages open via ms-settings: URIs.
const COMMANDS: &[(&str, &str, &str)] = &[
    ("zelo:settings", "Zelo Settings", "Nicknames and preferences"),
    ("media:playpause", "Play / Pause", "Media · works with YouTube, Spotify and more"),
    ("media:next", "Next Track", "Media"),
    ("media:prev", "Previous Track", "Media"),
    ("theme:dark", "Dark Mode", "Switch Windows to dark mode"),
    ("theme:light", "Light Mode", "Switch Windows to light mode"),
    ("lock", "Lock", "Lock this PC"),
    ("sleep", "Sleep", "Put the PC to sleep"),
    ("shutdown", "Shut Down", "Turn off the PC"),
    ("restart", "Restart", "Restart the PC"),
    ("ms-settings:", "Settings", "Open Windows Settings"),
    ("ms-settings:bluetooth", "Bluetooth", "Bluetooth settings"),
    ("ms-settings:network-wifi", "Wi-Fi", "Wi-Fi settings"),
    ("ms-settings:display", "Display", "Display settings"),
    ("ms-settings:sound", "Sound", "Sound settings"),
];

pub fn query(q: &str) -> Vec<SearchResult> {
    let m = SkimMatcherV2::default();
    COMMANDS
        .iter()
        .filter_map(|(id, title, sub)| {
            let s = m.fuzzy_match(title, q)?;
            Some(SearchResult {
                id: format!("cmd:{id}"),
                title: title.to_string(),
                subtitle: sub.to_string(),
                kind: ResultKind::Command,
                target: id.to_string(),
                score: s - 10, // slightly below apps with the same match
            })
        })
        .collect()
}

pub fn run(target: &str) -> Result<(), String> {
    let r = match target {
        "lock" => Command::new("rundll32.exe").args(["user32.dll,LockWorkStation"]).spawn(),
        "sleep" => Command::new("rundll32.exe").args(["powrprof.dll,SetSuspendState", "0,1,0"]).spawn(),
        "shutdown" => Command::new("shutdown").args(["/s", "/t", "0"]).spawn(),
        "restart" => Command::new("shutdown").args(["/r", "/t", "0"]).spawn(),
        "media:playpause" => return media_key(0xB3),
        "media:next" => return media_key(0xB0),
        "media:prev" => return media_key(0xB1),
        "theme:dark" => return set_theme(false),
        "theme:light" => return set_theme(true),
        uri => return open::that_detached(uri).map_err(|e| e.to_string()),
    };
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// Switch Windows between light and dark mode (apps + taskbar/Start).
fn set_theme(light: bool) -> Result<(), String> {
    let value = if light { "1" } else { "0" };
    for name in ["AppsUseLightTheme", "SystemUsesLightTheme"] {
        let mut cmd = Command::new("reg");
        cmd.args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v", name, "/t", "REG_DWORD", "/d", value, "/f",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // no console window flash
        }
        cmd.status().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Press a media key (same as the keyboard's play/pause/next buttons), so it controls
/// whatever is playing: YouTube in the browser, Spotify, etc.
#[cfg(windows)]
fn media_key(vk: u16) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), dwFlags: flags, ..Default::default() },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == 2 { Ok(()) } else { Err("couldn't send media key".into()) }
}

#[cfg(not(windows))]
fn media_key(_vk: u16) -> Result<(), String> {
    Ok(())
}
