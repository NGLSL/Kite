import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSearch } from "./useSearch";
import { SearchInput } from "./SearchInput";
import { ResultList } from "./ResultList";
import { StatusRegion } from "./StatusRegion";
import { SettingsPanel } from "./SettingsPanel";
import "./search.css";

export function SearchPanel() {
  const {
    query,
    setQuery,
    results,
    active,
    setActive,
    error,
    scanning,
    launch,
    moveActive,
    searchFiles,
    setSearchFiles,
  } = useSearch();
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [showSettings, setShowSettings] = useState(false);

  const focusSearch = useCallback(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  useEffect(() => {
    if (!showSettings) focusSearch();
  }, [showSettings, focusSearch]);

  useEffect(() => {
    const un = listen("kite://open-settings", () => setShowSettings(true));
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [active, results]);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Alt+1..9 直接启动对应结果
      if (e.altKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const idx = Number(e.key) - 1;
        if (idx < results.length) void launch(results[idx]);
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        moveActive(1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        moveActive(-1);
      } else if (e.key === "Enter") {
        e.preventDefault();
        void launch(results[active]);
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (showSettings) {
          setShowSettings(false);
          return;
        }
        setQuery("");
        void invoke("toggle_window");
      }
    },
    [active, launch, moveActive, results, setQuery, showSettings],
  );

  const empty = useMemo(
    () => !showSettings && query.trim().length > 0 && results.length === 0,
    [query, results, showSettings],
  );

  return (
    <div className="shell">
      <div className="panel">
        {showSettings ? (
          <SettingsPanel
            onClose={() => setShowSettings(false)}
            onRescanned={() => void invoke("rescan_apps")}
          />
        ) : (
          <>
            <SearchInput
              query={query}
              onQueryChange={setQuery}
              onKeyDown={onKeyDown}
              inputRef={inputRef}
              searchFiles={searchFiles}
              onToggleFiles={() => setSearchFiles((v) => !v)}
              onFocusSearch={focusSearch}
            />
            <div className="divider" />
            <StatusRegion scanning={scanning} empty={empty} error={error} />
            <ResultList
              results={results}
              active={active}
              query={query}
              listRef={listRef}
              onSelect={setActive}
              onLaunch={(item) => void launch(item)}
            />
          </>
        )}
      </div>
    </div>
  );
}
