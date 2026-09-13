/** 设置：Flow Launcher 风格 — 左侧导航 + 右侧条目卡片。 */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Settings } from "../../types/ipc";
import { startWindowDrag } from "../../shared/windowDrag";
import { NavIcon, Row, Toggle } from "./SettingsUI";
import { AboutSection } from "./AboutSection";
import { AliasSettings } from "./AliasSettings";

type Props = {
  onClose: () => void;
};

type SectionId = "general" | "hotkey" | "alias" | "index" | "about";

const HOTKEY_PRESETS = ["Alt+Space", "Ctrl+Alt+Space", "Ctrl+Shift+K", "Alt+K", "Ctrl+Alt+A"];

const NAV: { id: SectionId; label: string; icon: string }[] = [
  { id: "general", label: "通用", icon: "M4 6h16M4 12h16M4 18h10" },
  { id: "hotkey", label: "热键", icon: "M7 8h10M7 12h6M5 4h14a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H9l-4 3v-3H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z" },
  { id: "alias", label: "别名", icon: "M4 7h16M4 12h10M4 17h7M15 14l5 5m0-5l-5 5" },
  { id: "index", label: "应用索引", icon: "M4 6h16v12H4zM8 6v12M4 10h16M4 14h16" },
  { id: "about", label: "关于", icon: "M12 8h.01M11 12h1v4h1M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z" },
];

function modsFromEvent(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  return parts.join("+");
}

function keyFromEvent(e: KeyboardEvent): string | null {
  const k = e.key;
  if (["Control", "Alt", "Shift", "Meta"].includes(k)) return null;
  if (k === " ") return "Space";
  if (k.length === 1) {
    const c = k.toUpperCase();
    if (/^[A-Z0-9]$/.test(c)) return c;
    return null;
  }
  if (/^F([1-9]|1[0-2])$/.test(k)) return k;
  return null;
}

export function SettingsPanel({ onClose }: Props) {
  const [section, setSection] = useState<SectionId>("general");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const recordingRef = useRef(false);
  const msgTimer = useRef<number | undefined>(undefined);

  const flash = useCallback((text: string) => {
    setMsg(text);
    window.clearTimeout(msgTimer.current);
    msgTimer.current = window.setTimeout(() => setMsg(null), 2200);
  }, []);

  const reload = useCallback(async () => {
    const s = await invoke<Settings>("get_settings");
    setSettings(s);
  }, []);

  useEffect(() => {
    void reload().catch((e) => flash(String(e)));
  }, [reload, flash]);

  const save = async (
    patch: Partial<{
      hideOnBlur: boolean;
      autostart: boolean;
      hotkey: string;
      historyRecording: boolean;
    }>,
  ) => {
    try {
      // Tauri 默认把 JS camelCase 映射到 Rust snake_case
      const s = await invoke<Settings>("save_settings", patch);
      setSettings(s);
      flash("已保存");
    } catch (e) {
      flash(String(e));
      // 失败也回读，避免 UI 卡在旧状态
      void reload().catch(() => undefined);
    }
  };

  const applyHotkey = async (hotkey: string) => {
    setRecording(false);
    recordingRef.current = false;
    await save({ hotkey });
  };

  useEffect(() => {
    if (!recording) return;
    recordingRef.current = true;
    const onKey = (e: KeyboardEvent) => {
      if (!recordingRef.current) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        recordingRef.current = false;
        return;
      }
      const key = keyFromEvent(e);
      if (!key) return;
      const mods = modsFromEvent(e);
      if (!mods) {
        flash("请连同修饰键一起按下，例如 Ctrl+Alt+K");
        return;
      }
      void applyHotkey(`${mods}+${key}`);
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      recordingRef.current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording]);

  const clearHistory = async () => {
    try {
      await invoke("clear_history");
      flash("使用历史已清空");
    } catch (e) {
      flash(String(e));
    }
  };

  const rescan = async () => {
    // 只调用一次；完成后 Rust 广播 index-ready，搜索列表自动刷新
    const n = await invoke<number>("rescan_apps");
    flash(`已重新扫描，共 ${n} 个应用`);
  };

  if (!settings) {
    return (
      <div className="settings-flow">
        <div className="empty-state">加载设置…</div>
      </div>
    );
  }

  return (
    <div className="settings-flow">
      <aside className="settings-nav">
        <div className="nav-brand" data-tauri-drag-region onMouseDown={startWindowDrag}>
          <div className="nav-logo" aria-hidden>
            K
          </div>
          <div className="nav-brand-text" data-tauri-drag-region>
            <div className="nav-brand-name" data-tauri-drag-region>
              Kite
            </div>
            <div className="nav-brand-sub" data-tauri-drag-region>
              设置
            </div>
          </div>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="关闭设置">
            ×
          </button>
        </div>
        <nav className="nav-list">
          {NAV.map((n) => (
            <button
              key={n.id}
              type="button"
              className={`nav-item ${section === n.id ? "on" : ""}`}
              onClick={() => setSection(n.id)}
            >
              <NavIcon d={n.icon} />
              <span>{n.label}</span>
            </button>
          ))}
        </nav>
      </aside>

      <main className="settings-main">
        <header className="settings-main-head" data-tauri-drag-region onMouseDown={startWindowDrag}>
          <h2 data-tauri-drag-region>{NAV.find((n) => n.id === section)?.label}</h2>
        </header>

        <div className="settings-body">
          {section === "general" && (
            <div className="flow-card">
              <Row
                icon="M12 3v10M8 7l4-4 4 4M5 21h14"
                title="开机自动启动"
                hint="登录 Windows 后在后台待命"
              >
                <Toggle
                  checked={settings.autostart}
                  onChange={(v) => void save({ autostart: v })}
                />
              </Row>
              <Row
                icon="M4 4h16v12H4zM8 20h8"
                title="失焦时隐藏"
                hint="点击其它窗口后自动收起启动器"
              >
                <Toggle
                  checked={settings.hide_on_blur}
                  onChange={(v) => void save({ hideOnBlur: v })}
                />
              </Row>
              <Row
                icon="M12 4v16M5 9l7-5 7 5M5 15l7 5 7-5"
                title="记录使用历史"
                hint="暂停后不再记录启动次数与查询偏好"
              >
                <Toggle
                  checked={settings.history_recording}
                  onChange={(v) => void save({ historyRecording: v })}
                />
              </Row>
              <Row
                icon="M5 7h14M8 12h8M10 17h4"
                title="清空使用历史"
                hint="删除全部启动次数与查询配对，固定项不受影响"
              >
                <button type="button" className="btn" onClick={() => void clearHistory()}>
                  清空
                </button>
              </Row>
            </div>
          )}

          {section === "hotkey" && (
            <div className="flow-card">
              <Row icon="M7 8h10M7 12h6" title="全局快捷键" hint="按下组合键后立即生效">
                <div className="hotkey-inline">
                  <code className="hotkey-code">{settings.hotkey_label}</code>
                  <button
                    type="button"
                    className={`btn ${recording ? "btn-accent" : ""}`}
                    onClick={() => {
                      flash("请按下新的快捷键（Esc 取消）");
                      setRecording(true);
                    }}
                  >
                    {recording ? "按下组合键…" : "更改"}
                  </button>
                </div>
              </Row>
              <div className="flow-pad">
                <div className="set-hint">常用预设</div>
                <div className="chip-row">
                  {HOTKEY_PRESETS.map((h) => (
                    <button
                      key={h}
                      type="button"
                      className={`chip ${settings.hotkey === h ? "on" : ""}`}
                      onClick={() => void applyHotkey(h)}
                    >
                      {h}
                    </button>
                  ))}
                </div>
              </div>
            </div>
          )}

          {section === "alias" && <AliasSettings notify={flash} />}

          {section === "index" && (
            <div className="flow-card">
              <Row
                icon="M4 6h16v12H4zM9 6v12"
                title="重新扫描应用"
                hint="安装新软件后更新开始菜单与桌面索引"
              >
                <button type="button" className="btn" onClick={() => void rescan()}>
                  重新扫描
                </button>
              </Row>
            </div>
          )}

          {section === "about" && <AboutSection />}
        </div>
      </main>

      <div className={`settings-toast ${msg ? "show" : ""}`}>{msg}</div>
    </div>
  );
}
