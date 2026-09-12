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
  const [scanning, setScanning] = useState(true);
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

  // 事件 + 轮询双保险：避免 index-ready 在监听前就发完
  useEffect(() => {
    let cancelled = false;

    const syncFromIndex = async () => {
      try {
        const n = await invoke<number>("index_count");
        if (cancelled) return;
        if (n > 0) {
          setScanning(false);
          await runSearch(queryRef.current, filesRef.current);
          return true;
        }
      } catch {
        /* index_count 未就绪时忽略 */
      }
      return false;
    };

    const poll = async () => {
      for (let i = 0; i < 40 && !cancelled; i++) {
        const ok = await syncFromIndex();
        if (ok) return;
        await new Promise((r) => setTimeout(r, 150));
      }
      if (!cancelled) {
        // 仍无索引则停止扫描态，显示空结果，避免一直卡住
        setScanning(false);
      }
    };

    void poll();

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
      cancelled = true;
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
