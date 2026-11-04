import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface SemanticSettings {
  enabled: boolean;
  folders: string[];
}

interface Status {
  indexed: number;
  pending: number;
  running: boolean;
  model_missing: boolean;
  error: string | null;
}

export default function FileSearchCard() {
  const [settings, setSettings] = useState<SemanticSettings | null>(null);
  const [status, setStatus] = useState<Status | null>(null);

  useEffect(() => {
    invoke<SemanticSettings>("get_semantic_settings").then(setSettings);
    const poll = () => invoke<Status>("semantic_status").then(setStatus);
    poll();
    const t = setInterval(poll, 1500);
    return () => clearInterval(t);
  }, []);

  const save = (next: SemanticSettings) => {
    setSettings(next);
    invoke("set_semantic_settings", { settings: next });
  };

  const addFolder = async () => {
    if (!settings) return;
    const folder = await invoke<string | null>("pick_folder");
    if (folder && !settings.folders.includes(folder)) save({ ...settings, folders: [...settings.folders, folder] });
  };

  if (!settings) return null;

  let line = "";
  if (status?.model_missing) line = "Embedding model missing. Run: ollama pull nomic-embed-text";
  else if (status?.error) line = status.error;
  else if (status?.running) line = `Indexing… ${status.indexed} files done, ${status.pending} to go`;
  else if (status) line = `${status.indexed} files indexed`;

  return (
    <section className="card">
      <div className="list-head">
        <h2>
          Search inside files (optional)
          {status?.running && <span className="count learned">Indexing</span>}
          {(status?.error || status?.model_missing) && <span className="count warn-badge">Needs attention</span>}
        </h2>
        <label className="switch">
          <input type="checkbox" checked={settings.enabled} onChange={(e) => save({ ...settings, enabled: e.target.checked })} />
          <span>{settings.enabled ? "On" : "Off"}</span>
        </label>
      </div>
      <p className="muted">
        Off by default. When on, you can also type what a file is about, like “notes on neural networks” or “internship offer letter”, and Spotlight finds it
        even if the name doesn't match. Reads PDFs, Word, PowerPoint, text, Markdown and code. Everything stays on this
        PC.
      </p>
      <p className={`status-line ${status?.error || status?.model_missing ? "bad" : ""}`}>{line}</p>

      <ul className="folders">
        {settings.folders.map((f) => (
          <li key={f}>
            <span title={f}>{f}</span>
            <button className="link danger" onClick={() => save({ ...settings, folders: settings.folders.filter((x) => x !== f) })}>
              Remove
            </button>
          </li>
        ))}
        {settings.folders.length === 0 && <li className="empty">No folders yet.</li>}
      </ul>
      <div className="row">
        <button className="primary" onClick={addFolder}>
          Add folder
        </button>
        <button className="link" onClick={() => invoke("semantic_reindex")} disabled={status?.running}>
          Re-index now
        </button>
      </div>
      <p className="muted small">
        The first index can take a while for large folders (PDFs are slowest). It runs in the background and updates
        every 30 minutes; only new or changed files are re-read.
      </p>
    </section>
  );
}
