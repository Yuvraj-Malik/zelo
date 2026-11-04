# Spotlight for Windows

A fast, keyboard-first launcher for Windows, inspired by macOS Spotlight, with a private, on-device AI built in.

Press **Alt+Space**, type, press **Enter**.

Built with **Tauri 2 (Rust) + React + TypeScript**. The installer is about 4 MB.

---

## Features

### Search
- **Apps:** Start menu programs and Microsoft Store apps, with their real icons.
- **Files and folders by name** across all drives, with system folders skipped. Each search takes under a millisecond.
  - **Enter** opens the file. **Ctrl+Enter** shows it in Explorer.
- **Forgiving matching:** `vsc` finds Visual Studio Code, and words can be in any order.
- **Learns what you use:** frequently opened results rank higher.
- **Nicknames:** set your own (`ig` → Instagram), or let Spotlight learn them. If you pick the same result for the same short query three times, it saves that as a nickname.

### Instant answers (no AI needed)
- **Calculator:** `45*12`, `sqrt(144)`
- **Time and date:** `time`, `what day is it`, `time in tokyo`
- **Weather:** `weather`, `weather in delhi`, `will it rain tomorrow` (live, from Open-Meteo)

### Local AI (via [Ollama](https://ollama.com))
- **Natural-language actions:**
  - `remind me at 6 pm to call mom`
  - `set a timer for 10 minutes for tea`
  - `open youtube`
  - `turn on dark mode`
  - `launch visual studio code`
- **Answers in the bar:** `who is the ceo of google`. The answer streams in from a local model grounded in Wikipedia, and the source is shown.
- **Music:** `play goat by sidhu moose wala` opens and plays the video in your browser. Spotify and YouTube Music work too.
- **Optional search inside files:** find documents by what they're about, using local embeddings.

### More
- **Full-screen reminders** with a chime and snooze. Reminders survive restarts.
- **Media keys:** `play`, `pause`, `next`, `previous` control whatever is playing.
- **System commands:** lock, sleep, restart, dark/light mode, and Settings pages.
- **Starts with Windows,** runs in the tray, and only one copy runs at a time.
- **Private by design:** the AI runs on your PC. Only lookups such as Wikipedia, weather and YouTube go online.

---

## Install (for users)

1. Download `Spotlight_x.y.z_x64-setup.exe` from **[Releases](../../releases)** and run it.
   - If Windows shows "Windows protected your PC", click **More info → Run anyway**. The installer isn't code-signed.
2. Press **Alt+Space**. If another app already uses Alt+Space, Spotlight falls back to **Ctrl+Shift+Space**.
3. Optional, for the AI features:
   - Install [Ollama](https://ollama.com).
   - Run `ollama pull qwen2.5:3b`.
4. Optional, so `play …` starts the exact video:
   - Get a free YouTube Data API v3 key.
   - Paste it into **Settings → AI actions**.

Open **Settings** with **Ctrl+,**, by typing `settings`, or from the tray icon.

---

## Build from source

**Prerequisites**
- Node.js 18+
- Rust ([rustup.rs](https://rustup.rs))
- Visual Studio Build Tools with the **Desktop development with C++** workload
- WebView2 (already included in Windows 10/11)

```powershell
git clone https://github.com/Yuvraj-Malik/<repo-name>.git
cd <repo-name>
npm install --legacy-peer-deps
npx tauri icon app-icon.png    # generates the app icons
npm run tauri dev              # run in development
npm run tauri build            # build the installer
```

The installer is written to `src-tauri/target/release/bundle/nsis/`.

**API keys in development:** copy `.env.example` to `.env` and fill in `YOUTUBE_API_KEY`. The `.env` file is git-ignored. Installed builds read keys from Settings instead.

---

## Project structure

```
src/                         React UI
  App.tsx                    search bar, keyboard handling, result ordering
  Settings.tsx               settings window
  Reminder.tsx               full-screen reminder
  components/                result list, icons, answer card, settings sections
src-tauri/src/               Rust backend
  lib.rs                     startup, plugins, background workers
  commands.rs                commands the UI calls
  window.rs                  hotkey, tray, window placement
  providers/                 apps, calculator, system commands, web fallback
  files.rs                   whole-drive file name index
  semantic.rs                optional search inside files (local embeddings)
  ai.rs                      natural-language actions, answers, music
  instant.rs                 time, date, weather
  timeparse.rs               "6 pm", "in 20 minutes", "tomorrow morning"
  reminders.rs               reminders and timers
  aliases.rs                 nicknames (manual and learned)
  icons.rs                   app and file icon extraction and caching
```

## How it stays fast

- **No cold start:** the window is created once and only shown or hidden on the hotkey.
- **In memory:** apps, file names and icons are loaded into memory and saved to disk, so restarts are instant.
- **Never blocks typing:** local results appear immediately. AI, weather and file results arrive in the background without blocking typing, and out-of-date replies are discarded.

---

## Roadmap

- [ ] Quick Look-style preview pane
- [ ] Clipboard history
- [ ] Change the hotkey from Settings
- [ ] Unit and currency conversion
- [ ] Live file updates (watch the file system instead of rescanning)

## License

MIT
