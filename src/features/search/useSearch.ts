/** 搜索 IPC 与防抖 Query 状态。 */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { SearchResult } from "../../types/ipc";

const DEBOUNCE_MS = 40;

export function useSearch() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [active, setActive] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const debounceRef = useRef<number | undefined>(undefined);
  const queryRef = useRef("");
  queryRef.current = query;

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
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(debounceRef.current);
  }, [query, runSearch]);

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
      // 索引已就绪，立刻刷一次结果
      void runSearch(queryRef.current);
    });
    const unIcons = listen("kite://icons-ready", () => {
      // 图标补全后刷新，让列表显示图标
      void runSearch(queryRef.current);
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
        // query 一并传给 Rust，用于 Query History 配对记忆
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
  };
}
