//! Search files by meaning, fully on this PC.
//! A background thread reads documents in the chosen folders, turns each into an
//! embedding with Ollama's `nomic-embed-text`, and keeps them in memory. A query is
//! embedded the same way and compared against every file (cosine similarity).
use base64::Engine;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, UNIX_EPOCH},
};
use walkdir::WalkDir;

const OLLAMA: &str = "http://127.0.0.1:11434";
pub const EMBED_MODEL: &str = "nomic-embed-text";
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const TEXT_CHARS: usize = 2000;
const BATCH: usize = 16;
/// Below this similarity a file is probably unrelated.
const MIN_SCORE: f32 = 0.6;
const RESCAN_EVERY: Duration = Duration::from_secs(30 * 60);

const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", "venv", "env", "__pycache__",
    "AppData", "$RECYCLE.BIN", "site-packages", "bin", "obj",
];
const EXTENSIONS: &[&str] = &[
    "txt", "md", "pdf", "docx", "pptx", "csv", "tex", "rtf", "rs", "py", "ipynb", "js", "ts", "tsx",
    "jsx", "java", "c", "cpp", "h", "cs", "go", "kt", "html", "css", "sql",
];

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SemanticSettings {
    pub enabled: bool,
    pub folders: Vec<String>,
}

impl Default for SemanticSettings {
    fn default() -> Self {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let folders = ["Documents", "Desktop", "Downloads"]
            .iter()
            .map(|f| Path::new(&home).join(f))
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        // Off by default: most people search by name. Can be turned on in Settings.
        Self { enabled: false, folders }
    }
}

#[derive(Serialize, Clone, Default)]
pub struct Status {
    pub indexed: usize,
    pub pending: usize,
    pub running: bool,
    pub model_missing: bool,
    pub error: Option<String>,
}

struct Entry {
    path: String,
    mtime: u64,
    vec: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct StoredEntry {
    path: String,
    mtime: u64,
    /// Little-endian f32s, base64: about 4 KB per file instead of ~10 KB of JSON numbers.
    vec: String,
}

pub struct Semantic {
    dir: PathBuf,
    settings: Mutex<SemanticSettings>,
    entries: RwLock<Vec<Entry>>,
    status: Mutex<Status>,
    reindex: AtomicBool,
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn encode_vec(v: &[f32]) -> String {
    let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
    b64().encode(bytes)
}

fn decode_vec(s: &str) -> Option<Vec<f32>> {
    let bytes = b64().decode(s).ok()?;
    Some(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

fn mtime(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Batch-embed with Ollama. Vectors come back normalized, so dot product = cosine.
fn embed(texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let resp = ureq::post(&format!("{OLLAMA}/api/embed"))
        .timeout(Duration::from_secs(120))
        .send_json(json!({ "model": EMBED_MODEL, "input": texts, "truncate": true, "keep_alive": "10m" }));
    let v: Value = match resp {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(_, r)) => {
            let body = r.into_string().unwrap_or_default();
            return Err(body);
        }
        Err(e) => return Err(format!("Ollama isn't reachable: {e}")),
    };
    let arr = v["embeddings"].as_array().ok_or("no embeddings in reply")?;
    Ok(arr
        .iter()
        .map(|e| {
            normalize(
                e.as_array()
                    .map(|xs| xs.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect())
                    .unwrap_or_default(),
            )
        })
        .collect())
}

fn strip_xml(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() / 3);
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text from the XML parts of a .docx / .pptx (they are zip files).
fn zip_text(path: &Path, wanted: impl Fn(&str) -> bool) -> String {
    let Ok(file) = fs::File::open(path) else { return String::new() };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return String::new() };
    let mut text = String::new();
    for i in 0..zip.len() {
        let Ok(mut part) = zip.by_index(i) else { continue };
        if !wanted(part.name()) {
            continue;
        }
        let mut xml = String::new();
        if part.read_to_string(&mut xml).is_ok() {
            text.push_str(&strip_xml(&xml));
            text.push(' ');
        }
        if text.len() > TEXT_CHARS * 4 {
            break;
        }
    }
    text
}

fn extract_text(path: &Path, ext: &str) -> String {
    let text = match ext {
        "pdf" => {
            let p = path.to_path_buf();
            // Some PDFs make the parser panic; never let that take the indexer down.
            std::panic::catch_unwind(move || pdf_extract::extract_text(&p).unwrap_or_default()).unwrap_or_default()
        }
        "docx" => zip_text(path, |n| n == "word/document.xml"),
        "pptx" => zip_text(path, |n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml")),
        _ => {
            let mut buf = Vec::new();
            if let Ok(f) = fs::File::open(path) {
                let _ = f.take(64 * 1024).read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        }
    };
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TEXT_CHARS).collect()
}

fn wanted_dir(name: &str) -> bool {
    !name.starts_with('.') && !SKIP_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}

impl Semantic {
    pub fn load(dir: PathBuf) -> Self {
        let settings = fs::read_to_string(dir.join("content-search.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let entries: Vec<Entry> = fs::read_to_string(dir.join("semantic-index.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<StoredEntry>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| Some(Entry { vec: decode_vec(&e.vec)?, path: e.path, mtime: e.mtime }))
            .collect();
        let status = Status { indexed: entries.len(), ..Default::default() };
        Self {
            dir,
            settings: Mutex::new(settings),
            entries: RwLock::new(entries),
            status: Mutex::new(status),
            reindex: AtomicBool::new(false),
        }
    }

    pub fn settings(&self) -> SemanticSettings {
        self.settings.lock().clone()
    }

    pub fn set_settings(&self, s: SemanticSettings) {
        if let Ok(json) = serde_json::to_string_pretty(&s) {
            let _ = fs::write(self.dir.join("content-search.json"), json);
        }
        *self.settings.lock() = s;
        self.request_reindex();
    }

    pub fn status(&self) -> Status {
        self.status.lock().clone()
    }

    pub fn request_reindex(&self) {
        self.reindex.store(true, Ordering::Relaxed);
    }

    fn save(&self) {
        let stored: Vec<StoredEntry> = self
            .entries
            .read()
            .iter()
            .map(|e| StoredEntry { path: e.path.clone(), mtime: e.mtime, vec: encode_vec(&e.vec) })
            .collect();
        if let Ok(json) = serde_json::to_string(&stored) {
            let _ = fs::write(self.dir.join("semantic-index.json"), json);
        }
    }

    /// Background indexer: first pass shortly after startup, then every 30 minutes,
    /// or straight away when folders change / "Reindex" is pressed.
    pub fn start(self: Arc<Self>) {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(8));
            loop {
                if self.settings().enabled {
                    self.index_once();
                }
                let mut waited = Duration::ZERO;
                while waited < RESCAN_EVERY && !self.reindex.swap(false, Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_secs(2));
                    waited += Duration::from_secs(2);
                }
            }
        });
    }

    fn index_once(&self) {
        {
            let mut st = self.status.lock();
            st.running = true;
            st.error = None;
            st.model_missing = false;
        }
        // Is the embedding model there?
        if let Err(e) = embed(&["search_document: test".to_string()]) {
            let mut st = self.status.lock();
            st.running = false;
            st.model_missing = e.contains("not found");
            st.error = Some(if st.model_missing { format!("Run: ollama pull {EMBED_MODEL}") } else { e });
            return;
        }

        // Every candidate file with its modified time.
        let folders = self.settings().folders;
        let mut files: HashMap<String, u64> = HashMap::new();
        for folder in &folders {
            let walker = WalkDir::new(folder)
                .max_depth(8)
                .into_iter()
                .filter_entry(|e| !e.file_type().is_dir() || e.depth() == 0 || wanted_dir(&e.file_name().to_string_lossy()));
            for e in walker.filter_map(Result::ok) {
                if !e.file_type().is_file() {
                    continue;
                }
                let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                if !EXTENSIONS.contains(&ext.as_str()) {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                if meta.len() == 0 || meta.len() > MAX_FILE_BYTES {
                    continue;
                }
                files.insert(e.path().to_string_lossy().to_string(), mtime(&meta));
            }
        }

        // Keep unchanged entries, drop deleted ones, queue new/changed files.
        let todo: Vec<(String, u64)> = {
            let mut entries = self.entries.write();
            entries.retain(|e| files.get(&e.path) == Some(&e.mtime));
            let known: std::collections::HashSet<&str> = entries.iter().map(|e| e.path.as_str()).collect();
            files.iter().filter(|(p, _)| !known.contains(p.as_str())).map(|(p, m)| (p.clone(), *m)).collect()
        };
        {
            let mut st = self.status.lock();
            st.indexed = self.entries.read().len();
            st.pending = todo.len();
        }

        for (n, batch) in todo.chunks(BATCH).enumerate() {
            if self.reindex.load(Ordering::Relaxed) || !self.settings().enabled {
                break; // settings changed mid-way; the loop will restart us
            }
            let texts: Vec<String> = batch
                .iter()
                .map(|(p, _)| {
                    let path = Path::new(p);
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                    format!("search_document: {name}\n{}", extract_text(path, &ext))
                })
                .collect();
            match embed(&texts) {
                Ok(vecs) => {
                    let mut entries = self.entries.write();
                    for ((path, mtime), vec) in batch.iter().zip(vecs) {
                        entries.push(Entry { path: path.clone(), mtime: *mtime, vec });
                    }
                }
                Err(e) => {
                    self.status.lock().error = Some(e);
                    break;
                }
            }
            {
                let mut st = self.status.lock();
                st.indexed = self.entries.read().len();
                st.pending = st.pending.saturating_sub(batch.len());
            }
            if n % 10 == 9 {
                self.save();
            }
        }
        self.save();
        let mut st = self.status.lock();
        st.running = false;
        st.indexed = self.entries.read().len();
    }

    /// Best matching files for a query: (path, similarity 0..1).
    pub fn search(&self, q: &str, limit: usize) -> Vec<(String, f32)> {
        if !self.settings().enabled || self.entries.read().is_empty() {
            return vec![];
        }
        let Ok(mut v) = embed(&[format!("search_query: {q}")]) else { return vec![] };
        let Some(qv) = v.pop() else { return vec![] };
        let entries = self.entries.read();
        let mut scored: Vec<(String, f32)> = entries
            .iter()
            .filter(|e| e.vec.len() == qv.len())
            .map(|e| (e.path.clone(), e.vec.iter().zip(&qv).map(|(a, b)| a * b).sum::<f32>()))
            .filter(|(_, s)| *s >= MIN_SCORE)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
        scored
    }
}
