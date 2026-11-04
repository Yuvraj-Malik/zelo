import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Status {
  count: number;
  running: boolean;
  last_ms: number;
}

export default function NameIndexCard() {
  const [status, setStatus] = useState<Status | null>(null);

  useEffect(() => {
    const poll = () => invoke<Status>("file_index_status").then(setStatus);
    poll();
    const t = setInterval(poll, 1500);
    return () => clearInterval(t);
  }, []);

  const n = status?.count.toLocaleString() ?? "…";
  const line = status?.running
    ? `Scanning drives… (${n} files so far in the saved list)`
    : `${n} files and folders indexed${status?.last_ms ? ` · last scan took ${Math.round(status.last_ms / 1000)} s` : ""}`;

  return (
    <section className="card">
      <div className="list-head">
        <h2>
          Files by name
          {status?.running && <span className="count learned">Scanning</span>}
        </h2>
        <button className="link" disabled={status?.running} onClick={() => invoke("file_index_refresh")}>
          Rescan now
        </button>
      </div>
      <p className="muted">
        Type any part of a file or folder name. Enter opens it, Ctrl+Enter shows it in Explorer. Covers all your
        drives except system folders (Windows, Program Files, AppData, node_modules…). Rescans every 15 minutes.
      </p>
      <p className="status-line">{line}</p>
    </section>
  );
}
