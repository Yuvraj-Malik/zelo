import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function GeneralCard() {
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<boolean>("get_autostart").then(setAutostart).catch(() => setAutostart(false));
  }, []);

  const toggle = async (on: boolean) => {
    setError(null);
    try {
      await invoke("set_autostart", { enabled: on });
      setAutostart(on);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section className="card">
      <div className="list-head">
        <h2>General</h2>
      </div>
      <div className="row between">
        <div>
          <div className="setting-title">Start with Windows</div>
          <p className="muted small">Spotlight runs quietly in the tray so Ctrl+Shift+Space works right after you log in.</p>
        </div>
        <label className="switch">
          <input
            type="checkbox"
            checked={!!autostart}
            disabled={autostart === null}
            onChange={(e) => toggle(e.target.checked)}
          />
          <span>{autostart ? "On" : "Off"}</span>
        </label>
      </div>
      {error && <p className="status-line bad">{error}</p>}
    </section>
  );
}
