import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSearch } from "./useSearch";
import { SearchInput } from "./SearchInput";
import { ResultList } from "./ResultList";
import { ResultMenu, type MenuAction } from "./ResultMenu";
import { StatusRegion } from "./StatusRegion";
import { SettingsPanel } from "./SettingsPanel";
import { copyText } from "../../shared/clipboard";
import type { SearchResult } from "../../types/ipc";
import "./search.css";

export function SearchPanel() {
  const {
    query,
    setQuery,
    results,
    active,
    setActive,
    error,
    setError,
    scanning,
    launch,
    moveActive,
    searchFiles,
    toggleSearchFiles,
    pinnedIds,
    refreshPinned,
    refresh,
    loadMore,
  } = useSearch();
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [hotkeyLabel, setHotkeyLabel] = useState("Alt+Space");
  const [menu, setMenu] = useState<{ item: SearchResult; x: number; y: number } | null>(null);

  const focusSearch = useCallback(() => {
    const el = inputRef.current;
    if (!el) return;
    // WebView2 偶发丢焦点：窗口显示后下一帧再抢一次
    el.focus({ preventScroll: true });
    el.select();
  }, []);

  useEffect(() => {
    void invoke<{ hotkey_label: string }>("get_settings")
      .then((s) => setHotkeyLabel(s.hotkey_label))
      .catch(() => undefined);
    const un = listen("kite://settings-changed", () => {
      void invoke<{ hotkey_label: string }>("get_settings")
        .then((s) => setHotkeyLabel(s.hotkey_label))
        .catch(() => undefined);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // 打开/关闭设置时同步窗口尺寸与「设置中」标记（禁止失焦隐藏）
  const applyUiMode = useCallback((settings: boolean) => {
    void invoke("set_ui_mode", { settings }).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!showSettings) focusSearch();
    applyUiMode(showSettings);
  }, [showSettings, focusSearch, applyUiMode]);

  useEffect(() => {
    const unSettings = listen("kite://open-settings", () => {
      setShowSettings(true);
      applyUiMode(true);
    });
    const unCleared = listen("kite://cleared", () => {
      setShowSettings(false);
      applyUiMode(false);
    });
    // Alt+Space 显示窗口后必须把焦点交回输入框
    const unFocus = listen("kite://focus-search", () => {
      requestAnimationFrame(() => focusSearch());
      window.setTimeout(focusSearch, 30);
      window.setTimeout(focusSearch, 80);
    });
    return () => {
      unSettings.then((f) => f());
      unCleared.then((f) => f());
      unFocus.then((f) => f());
    };
  }, [focusSearch, applyUiMode]);

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [active, results]);

  // 滚动加载：距底部不足约一行时追加下一页
  const onResultsScroll = useCallback(
    (e: React.UIEvent<HTMLDivElement>) => {
      const el = e.currentTarget;
      if (el.scrollTop + el.clientHeight >= el.scrollHeight - 56) {
        void loadMore();
      }
    },
    [loadMore],
  );

  const closeMenu = useCallback(() => setMenu(null), []);

  const handleMenuAction = useCallback(
    async (a: MenuAction) => {
      if (!menu) return;
      const item = menu.item;
      setMenu(null);
      try {
        if (a === "open_folder") {
          await invoke("result_action", { id: item.id, action: "open_folder" });
        } else if (a === "copy_path") {
          const ok = await copyText(item.target);
          if (!ok) setError("复制失败");
        } else if (a === "copy_name") {
          const ok = await copyText(item.display_name || item.name);
          if (!ok) setError("复制失败");
        } else if (a === "pin" || a === "unpin") {
          await invoke("result_action", { id: item.id, action: a });
          void refreshPinned();
          refresh();
        }
      } catch (e) {
        setError(String(e));
      }
    },
    [menu, refresh, refreshPinned, setError],
  );

  const openMenuFor = useCallback((item: SearchResult, x: number, y: number) => {
    setMenu({ item, x, y });
  }, []);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Alt+1..9 直接启动对应结果（菜单打开时也保持可用）
      if (e.altKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const idx = Number(e.key) - 1;
        if (idx < results.length) void launch(results[idx]);
        return;
      }
      // Menu 键 / Shift+F10：为当前项打开上下文菜单
      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
        e.preventDefault();
        const item = results[active];
        const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
        if (item && el) {
          const r = el.getBoundingClientRect();
          setMenu({ item, x: r.left + 24, y: r.bottom - 6 });
        }
        return;
      }
      // 菜单打开时 ↑↓/Enter/Esc 由 ResultMenu 的 capture 处理，这里不再重复响应
      if (menu) return;
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
    [active, launch, menu, moveActive, results, setQuery, showSettings],
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
            onClose={() => {
              setShowSettings(false);
              refresh(); // Alias / 历史等设置变化立即反映到当前 Query
            }}
          />
        ) : (
          <>
            <SearchInput
              query={query}
              onQueryChange={setQuery}
              onKeyDown={onKeyDown}
              inputRef={inputRef}
              searchFiles={searchFiles}
              onToggleFiles={toggleSearchFiles}
            />
            <StatusRegion scanning={scanning} empty={empty} error={error} />
            <ResultList
              results={results}
              active={active}
              query={query}
              pinnedIds={pinnedIds}
              listRef={listRef}
              onSelect={setActive}
              onLaunch={(item) => void launch(item)}
              onContextMenu={openMenuFor}
              onScroll={onResultsScroll}
            />
            <div className="footer-bar">
              <span>
                <kbd>↑↓</kbd>选择
              </span>
              <span>
                <kbd>Enter</kbd>打开
              </span>
              <span>
                <kbd>Esc</kbd>关闭
              </span>
              <span className="footer-spacer" />
              <span className="footer-brand">{hotkeyLabel}</span>
            </div>
            {menu && (
              <ResultMenu
                item={menu.item}
                x={menu.x}
                y={menu.y}
                pinned={pinnedIds.has(menu.item.id)}
                onAction={(a) => void handleMenuAction(a)}
                onClose={closeMenu}
              />
            )}
          </>
        )}
      </div>
    </div>
  );
}
