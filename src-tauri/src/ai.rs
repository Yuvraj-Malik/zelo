//! Natural-language actions via a local Ollama model.
//! The model only picks from a fixed list of safe actions and must answer in a fixed
//! JSON shape; Rust then validates it, resolves apps and times, and builds results.
use crate::commands::{ResultKind, SearchResult};
use crate::providers::apps::AppEntry;
use crate::timeparse;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, time::Duration};

const OLLAMA: &str = "http://127.0.0.1:11434";
const KEEP_ALIVE: &str = "30m";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AiSettings {
    pub enabled: bool,
    pub model: String,
    /// Optional. Lets "play X on YouTube" start the top video instead of showing search results.
    pub youtube_api_key: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self { enabled: true, model: "qwen2.5:3b".into(), youtube_api_key: String::new() }
    }
}

pub struct AiConfig {
    pub settings: Mutex<AiSettings>,
    file: PathBuf,
}

impl AiConfig {
    pub fn load(dir: PathBuf) -> Self {
        let file = dir.join("ai.json");
        let settings = fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { settings: Mutex::new(settings), file }
    }

    pub fn get(&self) -> AiSettings {
        self.settings.lock().clone()
    }

    pub fn set(&self, s: AiSettings) {
        if let Ok(json) = serde_json::to_string_pretty(&s) {
            let _ = fs::write(&self.file, json);
        }
        *self.settings.lock() = s;
    }
}

/// Load the model into memory ahead of time so the first real query isn't a 30 s cold start.
pub fn warmup(model: &str) {
    let _ = ureq::post(&format!("{OLLAMA}/api/generate"))
        .timeout(Duration::from_secs(120))
        .send_json(json!({ "model": model, "prompt": "", "keep_alive": KEEP_ALIVE }));
}

pub fn list_models() -> Result<Vec<String>, String> {
    let v: Value = ureq::get(&format!("{OLLAMA}/api/tags"))
        .timeout(Duration::from_secs(3))
        .call()
        .map_err(|_| "Ollama isn't running".to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["models"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
        .unwrap_or_default())
}

const SYSTEM: &str = r#"You convert a command typed into a Windows app launcher into JSON. Pick exactly one action:
- open_app: open installed software. Fields: app (the app's name).
- open_url: open a website. Fields: url (domain or full URL).
- play_media: play or find music/videos. Fields: query (what to play, without the platform name; empty if none), platform (one of: youtube, spotify, youtube music; default youtube).
- answer: a question the user wants answered (facts, people, definitions, how/why). Fields: query (the question).
- web_search: the user explicitly wants to search the web or see results. Fields: query.
- reminder: remind the user later. Fields: message (what to remember, without the time), when (the time words exactly as the user wrote them).
- timer: a countdown. Fields: duration (e.g. "10 minutes"), message (what it is for, or empty).
- setting: change or open a Windows setting. Fields: setting (one of: dark mode, light mode, bluetooth, wifi, display, sound, battery, notifications, apps, updates, settings).
- none: it is not a command.
Prefer open_app for programs (Chrome, Spotify, VS Code, Word) and open_url for websites (YouTube, Gmail, GitHub).
Examples:
"remind me at 6 pm to call mom" -> {"action":"reminder","message":"call mom","when":"6 pm"}
"remind me in 20 minutes to drink water" -> {"action":"reminder","message":"drink water","when":"in 20 minutes"}
"set a timer for 10 minutes for pasta" -> {"action":"timer","duration":"10 minutes","message":"pasta"}
"launch visual studio code" -> {"action":"open_app","app":"Visual Studio Code"}
"open youtube" -> {"action":"open_url","url":"youtube.com"}
"play lofi on youtube" -> {"action":"play_media","query":"lofi","platform":"youtube"}
"play blinding lights on spotify" -> {"action":"play_media","query":"blinding lights","platform":"spotify"}
"watch mkbhd iphone review" -> {"action":"play_media","query":"mkbhd iphone review","platform":"youtube"}
"turn on dark mode" -> {"action":"setting","setting":"dark mode"}
"who won the last world cup" -> {"action":"answer","query":"who won the last world cup"}
"search for cheap flights to goa" -> {"action":"web_search","query":"cheap flights to goa"}
Reply with JSON only."#;

fn schema() -> Value {
    let s = json!({ "type": "string" });
    json!({
        "type": "object",
        "properties": {
            "action": { "type": "string", "enum": ["open_app", "open_url", "play_media", "answer", "web_search", "reminder", "timer", "setting", "none"] },
            "app": s, "url": s, "platform": s, "query": s, "message": s, "when": s, "duration": s, "setting": s
        },
        "required": ["action"]
    })
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
struct Intent {
    action: String,
    app: String,
    url: String,
    query: String,
    message: String,
    when: String,
    duration: String,
    setting: String,
    platform: String,
}

fn ask(model: &str, q: &str) -> Result<Intent, String> {
    let mut body = json!({
        "model": model,
        "stream": false,
        "format": schema(),
        "keep_alive": KEEP_ALIVE,
        "options": { "temperature": 0 },
        "messages": [
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": q }
        ]
    });
    // Thinking models (qwen3.x, deepseek-r1) are several seconds slower with thinking on.
    let m = model.to_lowercase();
    if m.starts_with("qwen3") || m.contains("deepseek-r1") {
        body["think"] = json!(false);
    }
    let v: Value = ureq::post(&format!("{OLLAMA}/api/chat"))
        .timeout(Duration::from_secs(30))
        .send_json(body)
        .map_err(|e| format!("Ollama request failed: {e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let content = v["message"]["content"].as_str().ok_or("Empty reply from model")?;
    serde_json::from_str(content).map_err(|e| format!("Bad JSON from model: {e}"))
}

fn result(title: String, detail: &str, kind: ResultKind, target: String) -> SearchResult {
    SearchResult {
        id: format!("ai:{target}"),
        title,
        subtitle: format!("✨ AI · {detail}"),
        kind,
        target,
        score: 3000,
    }
}

fn web_search(q: &str) -> SearchResult {
    let url = format!("https://www.google.com/search?q={}", urlencoding::encode(q));
    result(format!("Search the web for “{q}”"), "Google", ResultKind::Web, url)
}

/// "play lofi on spotify" -> Spotify app search (or the web player if the app isn't installed);
/// anything else -> YouTube / YouTube Music search results.
fn play_media(query: &str, platform: &str, apps: &[AppEntry], yt_key: &str) -> SearchResult {
    let p = platform.to_lowercase();
    let q = urlencoding::encode(query);
    if p.contains("spotify") {
        let has_app = apps.iter().any(|a| a.name.to_lowercase().contains("spotify"));
        let target = match (has_app, query.is_empty()) {
            (true, true) => "spotify:".to_string(),
            (true, false) => format!("spotify:search:{q}"),
            (false, true) => "https://open.spotify.com".to_string(),
            (false, false) => format!("https://open.spotify.com/search/{q}"),
        };
        let title = if query.is_empty() { "Open Spotify".into() } else { format!("Play “{query}” on Spotify") };
        return result(title, if has_app { "Spotify app" } else { "Spotify web" }, ResultKind::Web, target);
    }
    let music = p.contains("music");
    let (site, home, search) = if music {
        ("YouTube Music", "https://music.youtube.com", format!("https://music.youtube.com/search?q={q}"))
    } else {
        ("YouTube", "https://www.youtube.com", format!("https://www.youtube.com/results?search_query={q}"))
    };
    if query.is_empty() {
        return result(format!("Open {site}"), site, ResultKind::Web, home.into());
    }
    // With an API key: find the top video and open it directly, so it starts playing.
    if !yt_key.trim().is_empty() {
        if let Some((id, title)) = youtube_top_video(query, yt_key.trim()) {
            // Opens in your default browser and starts playing there.
            let watch = if music {
                format!("https://music.youtube.com/watch?v={id}")
            } else {
                format!("https://www.youtube.com/watch?v={id}")
            };
            return result(format!("Play “{title}”"), &format!("{site} · plays in your browser"), ResultKind::Web, watch);
        }
    }
    let note = if yt_key.trim().is_empty() {
        "search results · add a YouTube key in Settings to play directly"
    } else {
        "search results"
    };
    result(format!("Play “{query}” on {site}"), &format!("{site} · {note}"), ResultKind::Web, search)
}

/// YouTube Data API v3: top video for a query -> (video id, title).
fn youtube_top_video(query: &str, key: &str) -> Option<(String, String)> {
    let v: Value = ureq::get("https://www.googleapis.com/youtube/v3/search")
        .timeout(Duration::from_secs(4))
        .query("part", "snippet")
        .query("type", "video")
        .query("maxResults", "1")
        .query("q", query)
        .query("key", key)
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let item = v["items"].as_array()?.first()?;
    let id = item["id"]["videoId"].as_str()?.to_string();
    let title = item["snippet"]["title"].as_str().unwrap_or(query);
    // The API returns HTML entities in titles.
    let title = title.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"");
    Some((id, title))
}

fn setting_target(name: &str) -> (&'static str, &'static str) {
    let n = name.to_lowercase();
    let table: &[(&str, &str, &str)] = &[
        ("dark", "Turn on dark mode", "theme:dark"),
        ("light", "Turn on light mode", "theme:light"),
        ("bluetooth", "Open Bluetooth settings", "ms-settings:bluetooth"),
        ("wi", "Open Wi-Fi settings", "ms-settings:network-wifi"),
        ("display", "Open Display settings", "ms-settings:display"),
        ("bright", "Open Display settings", "ms-settings:display"),
        ("sound", "Open Sound settings", "ms-settings:sound"),
        ("volume", "Open Sound settings", "ms-settings:sound"),
        ("battery", "Open Battery settings", "ms-settings:batterysaver"),
        ("notification", "Open Notification settings", "ms-settings:notifications"),
        ("app", "Open Installed apps", "ms-settings:appsfeatures"),
        ("update", "Open Windows Update", "ms-settings:windowsupdate"),
    ];
    table
        .iter()
        .find(|(k, _, _)| n.contains(k))
        .map(|(_, t, u)| (*t, *u))
        .unwrap_or(("Open Settings", "ms-settings:"))
}

/// YouTube key: the one saved in Settings wins; otherwise YOUTUBE_API_KEY from the .env file.
fn youtube_key(cfg: &AiSettings) -> String {
    let saved = cfg.youtube_api_key.trim();
    if !saved.is_empty() {
        return saved.to_string();
    }
    std::env::var("YOUTUBE_API_KEY").unwrap_or_default().trim().to_string()
}

pub fn interpret(q: &str, cfg: &AiSettings, apps: &[AppEntry]) -> Result<Vec<SearchResult>, String> {
    let it = ask(&cfg.model, q)?;
    let r = match it.action.as_str() {
        "open_app" => {
            let name = if it.app.trim().is_empty() { q } else { it.app.trim() };
            let m = SkimMatcherV2::default();
            let best = apps
                .iter()
                .filter_map(|a| m.fuzzy_match(&a.name, name).map(|s| (s, a)))
                .max_by_key(|(s, _)| *s);
            match best {
                Some((_, a)) => result(format!("Open {}", a.name), "Application", ResultKind::App, a.path.clone()),
                None => web_search(name),
            }
        }
        "open_url" => {
            let mut url = it.url.trim().to_string();
            if url.is_empty() || url.contains(' ') {
                return Ok(vec![web_search(q)]);
            }
            if !url.contains("://") {
                url = format!("https://{url}");
            }
            let shown = url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
            result(format!("Open {shown}"), "Website", ResultKind::Web, url)
        }
        "play_media" => play_media(it.query.trim(), &it.platform, apps, &youtube_key(cfg)),
        "web_search" => web_search(if it.query.trim().is_empty() { q } else { it.query.trim() }),
        // The UI sees the "answer:" id and streams an inline answer instead of showing this row.
        "answer" => SearchResult {
            id: format!("answer:{q}"),
            title: q.to_string(),
            subtitle: "✨ AI · Answer".into(),
            kind: ResultKind::Web,
            target: format!("https://www.google.com/search?q={}", urlencoding::encode(q)),
            score: 3000,
        },
        "reminder" => {
            // Trust our own parser over the model: try its "when" first, then the whole sentence.
            let Some(at) = timeparse::parse(&it.when).or_else(|| timeparse::parse(q)) else {
                return Ok(vec![]);
            };
            let msg = if it.message.trim().is_empty() { q.to_string() } else { it.message.trim().to_string() };
            result(
                format!("Remind me: {msg}"),
                &timeparse::describe(at),
                ResultKind::Command,
                format!("remind:{}:{}", at.timestamp(), msg),
            )
        }
        "timer" => {
            let Some(secs) = timeparse::duration_secs(&it.duration).or_else(|| timeparse::duration_secs(q)) else {
                return Ok(vec![]);
            };
            let label = it.message.trim();
            let human = timeparse::human_duration(secs);
            let note = if label.is_empty() { format!("{human} timer is done") } else { format!("Timer done: {label}") };
            let at = chrono::Local::now().timestamp() + secs;
            let title = if label.is_empty() { format!("Start a {human} timer") } else { format!("Start a {human} timer for {label}") };
            result(title, "Timer", ResultKind::Command, format!("remind:{at}:{note}"))
        }
        "setting" => {
            let (title, target) = setting_target(&it.setting);
            result(title.to_string(), "Windows setting", ResultKind::Command, target.to_string())
        }
        _ => return Ok(vec![]),
    };
    Ok(vec![r])
}

// ---------------------------------------------------------------------------
// Inline answers: look the question up on Wikipedia, then let the model answer
// from that text (streamed), so facts are current and come with a source.
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct Source {
    pub title: String,
    pub url: String,
}

const UA: &str = "Zelo/0.1 (desktop launcher for Windows)";
const WIKI: &str = "https://en.wikipedia.org/w/api.php";

fn wiki_context(q: &str) -> Option<(String, Source)> {
    let search: Value = ureq::get(WIKI)
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(4))
        .query("action", "query")
        .query("list", "search")
        .query("srsearch", q)
        .query("srlimit", "2")
        .query("format", "json")
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let titles: Vec<String> = search["query"]["search"]
        .as_array()?
        .iter()
        .filter_map(|r| r["title"].as_str().map(String::from))
        .collect();
    let first = titles.first()?.clone();

    let pages: Value = ureq::get(WIKI)
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(4))
        .query("action", "query")
        .query("prop", "extracts")
        .query("exintro", "1")
        .query("explaintext", "1")
        .query("redirects", "1")
        .query("format", "json")
        .query("titles", &titles.join("|"))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let mut context = String::new();
    for page in pages["query"]["pages"].as_object()?.values() {
        let (Some(title), Some(text)) = (page["title"].as_str(), page["extract"].as_str()) else { continue };
        let text: String = text.chars().take(1500).collect();
        context.push_str(&format!("## {title}\n{text}\n\n"));
    }
    if context.is_empty() {
        return None;
    }
    let url = format!("https://en.wikipedia.org/wiki/{}", urlencoding::encode(&first.replace(' ', "_")));
    Some((context, Source { title: first, url }))
}

/// Streams the answer through `on_chunk`; returns the source it was based on, if any.
pub fn answer(model: &str, q: &str, mut on_chunk: impl FnMut(&str)) -> Result<Option<Source>, String> {
    use std::io::{BufRead, BufReader};

    let found = wiki_context(q);
    let today = chrono::Local::now().format("%-d %B %Y");
    let system = format!(
        "You answer questions typed into a desktop search bar. Today is {today}. \
         Answer in at most 2 short sentences, plain text, no preamble. \
         If context is given, rely on it over your own memory. \
         If you are not sure, say so in a few words."
    );
    let user = match &found {
        Some((ctx, _)) => format!("Context:\n{ctx}\nQuestion: {q}"),
        None => format!("Question: {q}"),
    };
    let mut body = json!({
        "model": model,
        "stream": true,
        "keep_alive": KEEP_ALIVE,
        "options": { "temperature": 0.2, "num_predict": 160 },
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
    let m = model.to_lowercase();
    if m.starts_with("qwen3") || m.contains("deepseek-r1") {
        body["think"] = json!(false);
    }
    let resp = ureq::post(&format!("{OLLAMA}/api/chat"))
        .timeout(Duration::from_secs(60))
        .send_json(body)
        .map_err(|e| format!("Ollama request failed: {e}"))?;
    for line in BufReader::new(resp.into_reader()).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(c) = v["message"]["content"].as_str() {
            if !c.is_empty() {
                on_chunk(c);
            }
        }
        if v["done"].as_bool() == Some(true) {
            break;
        }
    }
    Ok(found.map(|(_, src)| src))
}
