/** 搜索 IPC 与防抖 Query 状态。默认不搜文件。 */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { SearchResult } from "../../types/ipc";

const DEBOUNCE_MS = 40;
const DEBOUNCE_FILES_MS = 220;

export function useSearch() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [active, setActive] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [searchFiles, setSearchFiles] = useState(false);
  const debounceRef = useRef<number | undefined>(undefined);
  const queryRef = useRef("");
  const filesRef = useRef(false);
  queryRef.current = query;
  filesRef.current = searchFiles;

  const runSearch = useCallback(async (q: string, files: boolean) => {
    try {
      const hits = await invoke<SearchResult[]>("search_apps", {
        query: q,
        includeFiles: files,
      });
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
    const delay = searchFiles ? DEBOUNCE_FILES_MS : DEBOUNCE_MS;
    debounceRef.current = window.setTimeout(() => {
      void runSearch(query, searchFiles);
    }, delay);
    return () => window.clearTimeout(debounceRef.current);
  }, [query, searchFiles, runSearch]);

  useEffect(() => {
    setScanning(true);
    const unFocus = listen("kite://focus-search", () => {
      setQuery("");
      setActive(0);
    });
    const unCleared = listen("kite://cleared", () => {
      setQuery("");
      setActive(0);
    });
    const unReady = listen("kite://index-ready", () => {
      setScanning(false);
      void runSearch(queryRef.current, filesRef.current);
    });
    const unIcons = listen("kite://icons-ready", () => {
      void runSearch(queryRef.current, filesRef.current);
    });
    return () => {
      unFocus.then((f) => f());
      unCleared.then((f) => f());
      unReady.then((f) => f());
      unIcons.then((f) => f());
    };
  }, [runSearch]);

  const launch = useCallback(
    async (item: SearchResult | undefined) => {
      if (!item) return;
      try {
        await invoke("launch_app", { id: item.id, query });
      } catch (e) {
        setError(String(e));
      }
    },
    [query],
  );

  const moveActive = useCallback(
    (delta: number) => {
      setActive((i) => {
        if (!results.length) return 0;
        return (i + delta + results.length) % results.length;
      });
    },
    [results.length],
  );

  return {
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
  };
}
