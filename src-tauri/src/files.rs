//! Find any file or folder by name, like Spotlight on a Mac.
//! A background thread walks every drive (skipping system and junk folders), keeps all
//! names in memory, and saves the list so the next start is instant. Searching is a
//! plain in-memory scan, fast enough for every keystroke.
use parking_lot::{Mutex, RwLock};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    time::{Duration, Instant, UNIX_EPOCH},
};
use walkdir::WalkDir;

const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
/// Safety cap so a drive full of tiny files can't eat all memory.
const MAX_ENTRIES: usize = 1_500_000;

/// Folders nobody searches by name: Windows, installed programs, caches, build output.
const SKIP_DIRS: &[&str] = &[
    "windows", "program files", "program files (x86)", "programdata", "appdata", "$recycle.bin",
    "system volume information", "$winreagent", "recovery", "windowsapps", "wpsystem", "xboxgames",
    "steamapps", "msocache", "intel", "perflogs", "config.msi", "onedrivetemp",
    "node_modules", "__pycache__", "site-packages", "venv", "target", ".git", "bower_components",
];

pub struct Entry {
    pub path: String,
    /// Lower-cased file name, for matching.
    name_lower: String,
    pub is_dir: bool,
    pub mtime: u64,
}

#[derive(serde::Serialize, Clone, Default)]
pub struct Status {
    pub count: usize,
    pub running: bool,
    pub last_ms: u64,
}

pub struct FileIndex {
    cache: PathBuf,
    entries: RwLock<Arc<Vec<Entry>>>,
    status: Mutex<Status>,
    refresh: AtomicBool,
}

fn name_of(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn skip_dir(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with('.') || n.starts_with('$') || SKIP_DIRS.contains(&n.as_str())
}

fn skip_file(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("~$") || n == "desktop.ini" || n == "thumbs.db" || n.ends_with(".tmp") || n.ends_with(".sys")
}

/// Fixed drives that exist: C:\, D:\, ...
pub fn drives() -> Vec<PathBuf> {
    if cfg!(windows) {
        (b'C'..=b'Z')
            .map(|c| PathBuf::from(format!("{}:\\", c as char)))
            .filter(|p| p.is_dir())
            .collect()
    } else {
        vec![PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))]
    }
}

impl FileIndex {
    pub fn new(data_dir: PathBuf) -> Self {
        let cache = data_dir.join("file-index.txt");
        let entries = Self::load_cache(&cache);
        let status = Status { count: entries.len(), ..Default::default() };
        Self { cache, entries: RwLock::new(Arc::new(entries)), status: Mutex::new(status), refresh: AtomicBool::new(false) }
    }

    /// Cache format: one line per entry: "<d|f>\t<mtime>\t<path>"
    fn load_cache(path: &Path) -> Vec<Entry> {
        let Ok(f) = fs::File::open(path) else { return vec![] };
        BufReader::new(f)
            .lines()
            .filter_map(Result::ok)
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                let kind = p.next()?;
                let mtime = p.next()?.parse().ok()?;
                let path = p.next()?.to_string();
                Some(Entry { name_lower: name_of(&path).to_lowercase(), is_dir: kind == "d", mtime, path })
            })
            .collect()
    }

    fn save_cache(&self, entries: &[Entry]) {
        let tmp = self.cache.with_extension("tmp");
        let Ok(f) = fs::File::create(&tmp) else { return };
        let mut w = std::io::BufWriter::new(f);
        for e in entries {
            let _ = writeln!(w, "{}\t{}\t{}", if e.is_dir { 'd' } else { 'f' }, e.mtime, e.path);
        }
        drop(w);
        let _ = fs::rename(&tmp, &self.cache);
    }

    pub fn status(&self) -> Status {
        self.status.lock().clone()
    }

    pub fn request_refresh(&self) {
        self.refresh.store(true, Ordering::Relaxed);
    }

    pub fn start(self: Arc<Self>) {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            loop {
                self.rebuild();
                let mut waited = Duration::ZERO;
                while waited < REFRESH_EVERY && !self.refresh.swap(false, Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_secs(2));
                    waited += Duration::from_secs(2);
                }
            }
        });
    }

    fn rebuild(&self) {
        self.status.lock().running = true;
        let started = Instant::now();
        let mut out = Vec::with_capacity(200_000);
        'drives: for root in drives() {
            let walker = WalkDir::new(&root)
                .into_iter()
                .filter_entry(|e| e.depth() == 0 || !e.file_type().is_dir() || !skip_dir(&e.file_name().to_string_lossy()));
            for e in walker.filter_map(Result::ok) {
                if e.depth() == 0 {
                    continue;
                }
                let is_dir = e.file_type().is_dir();
                let name = e.file_name().to_string_lossy();
                if !is_dir && skip_file(&name) {
                    continue;
                }
                let mtime = e
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                out.push(Entry { name_lower: name.to_lowercase(), path: e.path().to_string_lossy().into_owned(), is_dir, mtime });
                if out.len() >= MAX_ENTRIES {
                    break 'drives;
                }
            }
        }
        self.save_cache(&out);
        let count = out.len();
        *self.entries.write() = Arc::new(out);
        let mut st = self.status.lock();
        *st = Status { count, running: false, last_ms: started.elapsed().as_millis() as u64 };
    }

    /// Best matches for `q`: every word must appear in the name. Ranked by how well
    /// the name matches, then recency, with a boost from `boost(path)` (usage).
    pub fn search(&self, q: &str, limit: usize, boost: impl Fn(&str) -> i64) -> Vec<(String, bool, u64)> {
        let q = q.trim().to_lowercase();
        if q.chars().count() < 2 {
            return vec![];
        }
        let words: Vec<&str> = q.split_whitespace().collect();
        let entries = self.entries.read().clone();
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        let mut hits: Vec<(i64, &Entry)> = entries
            .iter()
            .filter(|e| words.iter().all(|w| e.name_lower.contains(w)))
            .map(|e| {
                let n = &e.name_lower;
                let stem = n.rsplit_once('.').map(|(s, _)| s).unwrap_or(n);
                let mut score = if stem == q || n == &q {
                    1000
                } else if n.starts_with(&q) {
                    700
                } else if n.contains(&q) {
                    // whole query appears; better if it starts a word
                    let at_word = n.match_indices(&q).any(|(i, _)| i == 0 || !n.as_bytes()[i - 1].is_ascii_alphanumeric());
                    if at_word { 550 } else { 400 }
                } else {
                    250 // words found separately
                };
                score -= (n.len() as i64 - q.len() as i64).clamp(0, 60); // shorter names = closer match
                let age_days = now.saturating_sub(e.mtime) / 86_400;
                score += match age_days { 0..=1 => 60, 2..=7 => 40, 8..=30 => 20, _ => 0 };
                score += boost(&e.path);
                (score, e)
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0));
        hits.truncate(limit);
        hits.into_iter().map(|(_, e)| (e.path.clone(), e.is_dir, e.mtime)).collect()
    }
}
