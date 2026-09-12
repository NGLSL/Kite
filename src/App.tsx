import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { SearchResult } from "./types";
import "./App.css";

function iconSrc(item: SearchResult): string | undefined {
  if (!item.icon) return undefined;
  if (item.icon.startsWith("data:")) return item.icon;
  try {
    return convertFileSrc(item.icon);
  } catch {
    return undefined;
  }
}

function highlight(text: string, query: string) {
  const q = query.trim();
  if (!q) return text;
  const idx = text.toLowerCase().indexOf(q.toLowerCase());
  if (idx < 0) return text;
  return (
    <>
      {text.slice(0, idx)}
      <mark>{text.slice(idx, idx + q.length)}</mark>
      {text.slice(idx + q.length)}
    </>
  );
}

export default function App() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [active, setActive] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const debounceRef = useRef<number | undefined>(undefined);

  const focusSearch = useCallback(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  useEffect(() => {
    focusSearch();
    const unFocus = listen("kite://focus-search", () => {
      setQuery("");
      setActive(0);
      focusSearch();
    });
    const unCleared = listen("kite://cleared", () => {
      setQuery("");
      setActive(0);
    });
    const unReady = listen("kite://index-ready", () => setScanning(false));
    setScanning(true);
    return () => {
      unFocus.then((f) => f());
      unCleared.then((f) => f());
      unReady.then((f) => f());
    };
  }, [focusSearch]);

  const runSearch = useCallback(async (q: string) => {
    try {
      const hits = await invoke<SearchResult[]>("search_apps", { query: q });
      setResults(hits);
      setActive(0);
      setError(null);
    } catch (e) {
      setError(String(e));
      setResults([]);
    }
  }, []);

  useEffect(() => {
    window.clearTimeout(debounceRef.current);
    debounceRef.current = window.setTimeout(() => {
      void runSearch(query);
    }, 40);
    return () => window.clearTimeout(debounceRef.current);
  }, [query, runSearch]);

  const launch = useCallback(async (item: SearchResult | undefined) => {
    if (!item) return;
    try {
      await invoke("launch_app", { id: item.id });
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [active, results]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (results.length ? (i + 1) % results.length : 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (results.length ? (i - 1 + results.length) % results.length : 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      void launch(results[active]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setQuery("");
      void invoke("toggle_window");
    }
  };

  const empty = useMemo(() => query.trim().length > 0 && results.length === 0, [query, results]);

  return (
    <div className="shell">
      <div className="panel">
        <div className="search-row">
          <svg className="search-icon" viewBox="0 0 24 24" aria-hidden>
            <circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" strokeWidth="1.8" />
            <path d="M20 20l-3.5-3.5" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          </svg>
          <input
            ref={inputRef}
            className="search-input"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Search..."
            spellCheck={false}
            autoComplete="off"
            aria-label="Search applications"
          />
          {query && (
            <button className="clear-btn" onClick={() => { setQuery(""); focusSearch(); }} aria-label="Clear">
              ×
            </button>
          )}
        </div>

        <div className="divider" />

        {scanning && results.length === 0 && !empty && (
          <div className="empty-state">正在扫描应用…</div>
        )}

        {empty && (
          <div className="empty-state">
            <div className="empty-title">没有找到相关结果</div>
            <div className="empty-sub">试试其他关键词</div>
          </div>
        )}

        {error && <div className="error-banner">{error}</div>}

        <div className="results" ref={listRef} role="listbox">
          {results.map((item, i) => {
            const src = iconSrc(item);
            return (
              <div
                key={item.id}
                data-index={i}
                role="option"
                aria-selected={i === active}
                className={`item ${i === active ? "active" : ""}`}
                onMouseMove={() => setActive(i)}
                onClick={() => void launch(item)}
              >
                <div className="item-icon">
                  {src ? (
                    <img src={src} alt="" width={32} height={32} draggable={false} />
                  ) : (
                    <div className="icon-fallback">{item.name.slice(0, 1).toUpperCase()}</div>
                  )}
                </div>
                <div className="item-body">
                  <div className="item-name">{highlight(item.display_name || item.name, query)}</div>
                  <div className="item-path" title={item.target}>
                    {item.target}
                  </div>
                </div>
                <div className="item-hint">Alt+{i + 1}</div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
