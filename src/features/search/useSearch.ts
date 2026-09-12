/** 搜索 IPC 与防抖 Query 状态。默认不搜文件；结果滚动加载（Rust 按条数切片返回）。 */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { SearchResult } from "../../types/ipc";

const DEBOUNCE_MS = 40;
const DEBOUNCE_FILES_MS = 220;
/** 与 search.css 对齐：结果行 min-height 52 + gap 2 */
const ROW_PX = 54;
/** 输入行 + 底栏 + 留白的估算高度 */
const CHROME_PX = 110;
/** 每次滚动到底部追加的条数 */
const PAGE_STEP = 10;
/** 首屏条数上下限 */
const MIN_COUNT = 6;
const MAX_COUNT = 30;

/** 按软件窗口高度动态计算首屏加载条数（可见行数 + 少量预载） */
function initialCount(): number {
  const rows = Math.ceil(Math.max(window.innerHeight - CHROME_PX, ROW_PX) / ROW_PX);
  return Math.min(MAX_COUNT, Math.max(MIN_COUNT, rows + 2));
}

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

  // 滚动加载：已请求条数 / 结果镜像 / 在途标记
  const requestedRef = useRef(initialCount());
  const resultsRef = useRef<SearchResult[]>([]);
  const loadingMoreRef = useRef(false);

  const runSearch = useCallback(async (q: string, files: boolean) => {
    try {
      const want = initialCount(); // 新查询重置为首屏条数
      requestedRef.current = want;
      const hits = await invoke<SearchResult[]>("search_apps", {
        query: q,
        includeFiles: files,
        limit: want,
      });
      resultsRef.current = hits;
      setResults(hits);
      setActive(0);
      setError(null);
    } catch (e) {
      setError(String(e));
      setResults([]);
    }
  }, []);

  /** 滚动到底部时请求更多。到底（上次返回不足）或在途则忽略；
   *  只替换列表不动 active——前缀稳定，已渲染行复用，滚动位置保持。 */
  const loadMore = useCallback(async () => {
    if (loadingMoreRef.current) return;
    if (resultsRef.current.length < requestedRef.current) return;
    loadingMoreRef.current = true;
    const want = requestedRef.current + PAGE_STEP;
    try {
      const hits = await invoke<SearchResult[]>("search_apps", {
        query: queryRef.current,
        includeFiles: filesRef.current,
        limit: want,
      });
      requestedRef.current = want;
      resultsRef.current = hits;
      setResults(hits);
    } catch {
      /* 静默：保留当前结果，下次滚动再试 */
    } finally {
      loadingMoreRef.current = false;
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
      for (let i = 0; i < 20 && !cancelled; i++) {
        const ok = await syncFromIndex();
        if (ok) return;
        await new Promise((r) => setTimeout(r, 120));
      }
      if (!cancelled) {
        setScanning(false);
        void runSearch(queryRef.current, filesRef.current);
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
    loadMore,
  };
}
