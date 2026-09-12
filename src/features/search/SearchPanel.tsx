import { useCallback, useEffect, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useSearch } from "./useSearch";
import { SearchInput } from "./SearchInput";
import { ResultList } from "./ResultList";
import { StatusRegion } from "./StatusRegion";
import "./search.css";

export function SearchPanel() {
  const { query, setQuery, results, active, setActive, error, scanning, launch, moveActive } =
    useSearch();
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const focusSearch = useCallback(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  useEffect(() => {
    focusSearch();
  }, [focusSearch]);

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [active, results]);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
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
        setQuery("");
        void invoke("toggle_window");
      }
    },
    [active, launch, moveActive, results, setQuery],
  );

  const empty = useMemo(
    () => query.trim().length > 0 && results.length === 0,
    [query, results],
  );

  return (
    <div className="shell">
      <div className="panel">
        <SearchInput
          query={query}
          onQueryChange={setQuery}
          onKeyDown={onKeyDown}
          inputRef={inputRef}
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
      </div>
    </div>
  );
}
