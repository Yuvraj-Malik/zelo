import { useEffect, useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import SearchBar from "./components/SearchBar";
import ResultList from "./components/ResultList";
import AnswerCard, { type AnswerSource } from "./components/AnswerCard";
import type { SearchResult } from "./types";

/** Sentences of this many words go to the AI automatically; shorter queries only on Tab. */
const AI_MIN_WORDS = 3;
const AI_DEBOUNCE_MS = 400;

/** Two-word commands like "open youtube" are worth sending to the AI too. */
const COMMAND_VERBS =
  /^(open|launch|start|run|go to|goto|play|watch|listen|search|find|google|look up|show|check|remind|set|turn|switch|enable|disable|timer|mute|unmute)\b/i;

const WINDOW_WIDTH = 680;
const SHELL_MARGIN = 16; // .shell has an 8px margin on each side

/** Looks like a question → skip intent detection and answer it directly. */
function isQuestion(q: string) {
  const t = q.trim().toLowerCase();
  if (t.split(/\s+/).length < 2) return false;
  return (
    t.endsWith("?") ||
    /^(who|what|whats|what's|when|where|why|how|which|whose|is|are|was|were|does|did|can|could|should|define|explain|tell me)\b/.test(t)
  );
}

interface Answer {
  text: string;
  loading: boolean;
  source: AnswerSource | null;
  error: string | null;
}

export default function App() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [aiResults, setAiResults] = useState<SearchResult[]>([]);
  const [fileResults, setFileResults] = useState<SearchResult[]>([]);
  const [instantResults, setInstantResults] = useState<SearchResult[]>([]);
  const instantRequestId = useRef(0);

  // Time, date and weather: answered directly, no AI.
  useEffect(() => {
    const id = ++instantRequestId.current;
    setInstantResults([]);
    if (!query.trim()) return;
    const t = setTimeout(() => {
      invoke<SearchResult[]>("instant_answer", { query })
        .then((r) => {
          if (id !== instantRequestId.current) return;
          setInstantResults(r);
          if (r.length) setSelected(0);
        })
        .catch(() => {});
    }, 120);
    return () => clearTimeout(t);
  }, [query]);
  const [nameResults, setNameResults] = useState<SearchResult[]>([]);
  const nameRequestId = useRef(0);

  // Files and folders by name, from the in-memory index of your drives.
  useEffect(() => {
    const id = ++nameRequestId.current;
    setNameResults([]);
    if (query.trim().length < 2) return;
    const t = setTimeout(() => {
      invoke<SearchResult[]>("file_search", { query })
        .then((r) => id === nameRequestId.current && setNameResults(r))
        .catch(() => {});
    }, 60);
    return () => clearTimeout(t);
  }, [query]);
  const fileRequestId = useRef(0);
  const [thinking, setThinking] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [selected, setSelected] = useState(0);
  const requestId = useRef(0);
  const aiRequestId = useRef(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const shellRef = useRef<HTMLDivElement>(null);

  // Resize the native window to fit the content, so there's no empty area below.
  useEffect(() => {
    const el = shellRef.current;
    if (!el) return;
    const win = getCurrentWindow();
    let last = 0;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.getBoundingClientRect().height) + SHELL_MARGIN;
      if (h !== last) {
        last = h;
        win.setSize(new LogicalSize(WINDOW_WIDTH, h));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => {
    const un = listen<[string, string]>("zelo://alias-learned", (e) => {
      const [alias, title] = e.payload;
      setToast(`Learned: “${alias}” → ${title}`);
      setTimeout(() => setToast(null), 4000);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Instant local search on every keystroke; stale responses are dropped.
  useEffect(() => {
    const id = ++requestId.current;
    if (!query.trim()) {
      setResults([]);
      return;
    }
    invoke<SearchResult[]>("search", { query }).then((r) => {
      if (id === requestId.current) {
        setResults(r);
        setSelected(0);
      }
    });
  }, [query]);

  const runAnswer = (q: string, id: number) => {
    setThinking(false);
    setAnswer({ text: "", loading: true, source: null, error: null });
    const ch = new Channel<string>();
    ch.onmessage = (chunk) => {
      if (id === aiRequestId.current) setAnswer((a) => a && { ...a, text: a.text + chunk });
    };
    invoke<AnswerSource | null>("ai_answer", { query: q, onChunk: ch })
      .then((source) => {
        if (id === aiRequestId.current) setAnswer((a) => a && { ...a, loading: false, source });
      })
      .catch((err) => {
        if (id === aiRequestId.current) setAnswer((a) => a && { ...a, loading: false, error: String(err) });
      });
  };

  const runAi = (q: string, explicit: boolean) => {
    const id = ++aiRequestId.current;
    setAiError(null);
    if (isQuestion(q)) {
      runAnswer(q, id);
      return;
    }
    setThinking(true);
    invoke<SearchResult[]>("ai_interpret", { query: q })
      .then((r) => {
        if (id !== aiRequestId.current) return;
        if (r.some((x) => x.id.startsWith("answer:"))) {
          runAnswer(q, id);
          return;
        }
        setAiResults(r);
        if (r.length) setSelected(0);
      })
      .catch((err) => {
        // Only bother the user about AI problems when they asked for AI with Tab.
        if (id === aiRequestId.current && explicit) setAiError(String(err));
      })
      .finally(() => {
        if (id === aiRequestId.current) setThinking(false);
      });
  };

  // AI runs in the background for sentences, after a short pause in typing.
  useEffect(() => {
    aiRequestId.current++;
    setAiResults([]);
    setAnswer(null);
    setThinking(false);
    setAiError(null);
    const words = query.trim().split(/\s+/).filter(Boolean).length;
    if (words < AI_MIN_WORDS && !(words >= 2 && COMMAND_VERBS.test(query.trim()))) return;
    const t = setTimeout(() => runAi(query, false), AI_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [query]);

  // Search files by meaning (local embeddings) for any query of 2+ words.
  useEffect(() => {
    const id = ++fileRequestId.current;
    setFileResults([]);
    const words = query.trim().split(/\s+/).filter(Boolean).length;
    if (words < 2) return;
    // General questions ("what is the time") aren't about your files; skip unless they mention one.
    if (isQuestion(query) && !/\b(files?|pdfs?|docs?|documents?|notes|ppts?|slides|assignments?|reports?)\b/i.test(query)) return;
    const t = setTimeout(() => {
      invoke<SearchResult[]>("semantic_search", { query })
        .then((r) => id === fileRequestId.current && setFileResults(r))
        .catch(() => {});
    }, 250);
    return () => clearTimeout(t);
  }, [query]);

  // Order: AI action, apps/commands, files by name, files by content, then the web fallback.
  const combined = useMemo(() => {
    const seen = new Set<string>();
    const local = results.filter((r) => r.id !== "web");
    const web = results.filter((r) => r.id === "web");
    // A direct answer (time, weather) is the result; keep the rest out of the way.
    if (instantResults.length) {
      return [...instantResults, ...web].filter((r) => !seen.has(r.id) && !!seen.add(r.id));
    }
    return [...aiResults, ...local, ...nameResults, ...fileResults, ...web].filter((r) => !seen.has(r.id) && !!seen.add(r.id));
  }, [aiResults, results, nameResults, fileResults, instantResults]);

  // Reset and refocus each time the window is shown.
  useEffect(() => {
    const unlisten = listen("zelo://shown", () => {
      setQuery("");
      inputRef.current?.focus();
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const run = async (r?: SearchResult) => {
    if (!r) return;
    try {
      await invoke("execute", { result: r, query });
      await invoke("hide_window");
    } catch (err) {
      // Never fail silently: show what went wrong in the bar.
      setAiError(`Couldn't open that: ${err}`);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((s) => Math.min(s + 1, visible.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter" && e.ctrlKey) {
      // On a file: show it in Explorer. Otherwise: search Google.
      const cur = visible[selected];
      if (cur?.kind === "file") {
        invoke("reveal_file", { path: cur.target });
        invoke("hide_window");
      } else {
        run(results.find((r) => r.id === "web"));
      }
    } else if (e.key === "Enter") {
      run(combined[selected]);
    } else if ((e.key === "c" || e.key === "C") && e.ctrlKey && answer?.text) {
      const input = inputRef.current;
      const hasSelection = input && input.selectionStart !== input.selectionEnd;
      if (!hasSelection) {
        e.preventDefault();
        navigator.clipboard.writeText(answer.text.trim());
      }
    } else if (e.key === "Tab") {
      e.preventDefault();
      if (query.trim() && !thinking && !answer?.loading) runAi(query, true);
    } else if (e.key === "," && e.ctrlKey) {
      e.preventDefault();
      invoke("open_settings");
      invoke("hide_window");
    } else if (e.key === "Escape") {
      invoke("hide_window");
    }
  };

  const showList = combined.length > 0 || thinking || aiError || answer;
  // Keep the window from overflowing when an answer is shown.
  const visible = answer ? combined.slice(0, 4) : combined;
  const openUrl = (url: string) =>
    run({ id: `src:${url}`, title: url, subtitle: "", kind: "web", target: url, score: 0 });

  return (
    <div className="shell" ref={shellRef} onKeyDown={onKeyDown}>
      <SearchBar ref={inputRef} value={query} onChange={setQuery} />
      {toast && !query && <div className="toast">{toast}</div>}
      {showList && (
        <>
          {thinking && (
            <div className="ai-status">
              <span className="spark">✨</span> Thinking…
            </div>
          )}
          {aiError && <div className="ai-status error">AI unavailable: {aiError}</div>}
          {answer && instantResults.length === 0 && <AnswerCard {...answer} onOpenSource={openUrl} />}
          {visible.length > 0 && (
            <ResultList results={visible} selected={selected} onHover={setSelected} onPick={run} />
          )}
        </>
      )}
      {query.trim() && !thinking && !answer && aiResults.length === 0 && instantResults.length === 0 && (
        <div className="hint">Tab to ask AI</div>
      )}
    </div>
  );
}
