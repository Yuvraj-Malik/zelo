import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Alias, SearchResult } from "./types";
import AiSettingsCard from "./components/AiSettingsCard";
import GeneralCard from "./components/GeneralCard";
import FileSearchCard from "./components/FileSearchCard";
import NameIndexCard from "./components/NameIndexCard";

export default function Settings() {
  const [aliases, setAliases] = useState<Alias[]>([]);
  const [nickname, setNickname] = useState("");
  const [targetQuery, setTargetQuery] = useState("");
  const [matches, setMatches] = useState<SearchResult[]>([]);
  const [target, setTarget] = useState<SearchResult | null>(null);
  const [filter, setFilter] = useState("");

  const refresh = () => invoke<Alias[]>("list_aliases").then(setAliases);

  useEffect(() => {
    refresh();
    // Pick up nicknames learned while this window was in the background.
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);

  useEffect(() => {
    if (!targetQuery.trim() || target) {
      setMatches([]);
      return;
    }
    invoke<SearchResult[]>("search", { query: targetQuery }).then((r) =>
      setMatches(r.filter((x) => x.kind !== "calc" && x.kind !== "web").slice(0, 6))
    );
  }, [targetQuery, target]);

  const save = async () => {
    if (!nickname.trim() || !target) return;
    await invoke("set_alias", { alias: nickname, result: target });
    setNickname("");
    setTargetQuery("");
    setTarget(null);
    refresh();
  };

  const remove = async (alias: string) => {
    await invoke("remove_alias", { alias });
    refresh();
  };

  const clash = aliases.find((a) => a.alias === nickname.trim().toLowerCase());
  const shown = aliases.filter(
    (a) => !filter || a.alias.includes(filter.toLowerCase()) || a.result.title.toLowerCase().includes(filter.toLowerCase())
  );
  const learnedCount = aliases.filter((a) => a.auto).length;

  return (
    <div className="settings">
      <header>
        <h1>Zelo Settings</h1>
        <p>
          Give anything a short name. Zelo also learns them on its own: pick the same result for the same
          short query 3 times and it becomes a nickname.
        </p>
      </header>

      <GeneralCard />
      <AiSettingsCard />
      <NameIndexCard />
      <FileSearchCard />

      <section className="card">
        <h2>Add a nickname</h2>
        <div className="row">
          <input
            className="nick"
            placeholder="Nickname, e.g. ig"
            value={nickname}
            onChange={(e) => setNickname(e.target.value)}
          />
          <span className="arrow">→</span>
          <div className="picker">
            {target ? (
              <div className="chosen">
                <span>{target.title}</span>
                <button className="link" onClick={() => setTarget(null)}>change</button>
              </div>
            ) : (
              <input
                placeholder="Search for an app or command"
                value={targetQuery}
                onChange={(e) => setTargetQuery(e.target.value)}
              />
            )}
            {matches.length > 0 && (
              <ul className="dropdown">
                {matches.map((m) => (
                  <li key={m.id} onClick={() => setTarget(m)}>
                    <span>{m.title}</span>
                    <small>{m.subtitle}</small>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <button className="primary" disabled={!nickname.trim() || !target} onClick={save}>
            {clash ? "Replace" : "Save"}
          </button>
        </div>
        {clash && <p className="warn">“{clash.alias}” already points to {clash.result.title}. Saving will replace it.</p>}
      </section>

      <section className="card">
        <div className="list-head">
          <h2>
            Your nicknames <span className="count">{aliases.length}</span>
            {learnedCount > 0 && <span className="count learned">{learnedCount} learned</span>}
          </h2>
          <input className="filter" placeholder="Filter" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </div>
        {shown.length === 0 ? (
          <p className="empty">
            {aliases.length === 0 ? "No nicknames yet. Add one above, or just keep using Zelo." : "No matches."}
          </p>
        ) : (
          <table>
            <tbody>
              {shown.map((a) => (
                <tr key={a.alias}>
                  <td className="alias">{a.alias}</td>
                  <td>
                    {a.result.title}
                    <small>{a.result.kind}</small>
                  </td>
                  <td>{a.auto ? <span className="badge auto">Learned</span> : <span className="badge">Manual</span>}</td>
                  <td className="actions">
                    <button className="link danger" onClick={() => remove(a.alias)}>Delete</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
