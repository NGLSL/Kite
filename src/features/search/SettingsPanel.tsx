/** 设置：开机启动、失焦隐藏、结果数、用户 Alias、重扫。 */

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Settings, UserAlias } from "../../types/ipc";

type Props = {
  onClose: () => void;
  onRescanned?: () => void;
};

export function SettingsPanel({ onClose, onRescanned }: Props) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [aliases, setAliases] = useState<UserAlias[]>([]);
  const [alias, setAlias] = useState("");
  const [target, setTarget] = useState("");
  const [msg, setMsg] = useState<string | null>(null);

  const reload = useCallback(async () => {
    const s = await invoke<Settings>("get_settings");
    setSettings(s);
    setAliases(await invoke<UserAlias[]>("list_user_aliases"));
  }, []);

  useEffect(() => {
    void reload().catch((e) => setMsg(String(e)));
  }, [reload]);

  const save = async (patch: Partial<{ hide_on_blur: boolean; autostart: boolean; max_results: number }>) => {
    try {
      const s = await invoke<Settings>("save_settings", patch);
      setSettings(s);
      setMsg("已保存");
    } catch (e) {
      setMsg(String(e));
    }
  };

  const addAlias = async () => {
    if (!alias.trim() || !target.trim()) return;
    try {
      const list = await invoke<UserAlias[]>("set_user_alias", {
        alias: alias.trim(),
        targetName: target.trim(),
      });
      setAliases(list);
      setAlias("");
      setTarget("");
      setMsg("Alias 已添加");
    } catch (e) {
      setMsg(String(e));
    }
  };

  const removeAlias = async (a: string) => {
    const list = await invoke<UserAlias[]>("remove_user_alias", { alias: a });
    setAliases(list);
  };

  const rescan = async () => {
    const n = await invoke<number>("rescan_apps");
    setMsg(`已重新扫描，共 ${n} 个应用`);
    onRescanned?.();
  };

  if (!settings) {
    return <div className="empty-state">加载设置…</div>;
  }

  return (
    <div className="settings">
      <div className="settings-head">
        <h2>设置</h2>
        <button type="button" className="clear-btn" onClick={onClose} aria-label="关闭">
          ×
        </button>
      </div>

      <label className="row">
        <input
          type="checkbox"
          checked={settings.hide_on_blur}
          onChange={(e) => void save({ hide_on_blur: e.currentTarget.checked })}
        />
        失焦时隐藏窗口
      </label>

      <label className="row">
        <input
          type="checkbox"
          checked={settings.autostart}
          onChange={(e) => void save({ autostart: e.currentTarget.checked })}
        />
        开机自动启动
      </label>

      <label className="row">
        结果数量
        <input
          type="number"
          min={5}
          max={30}
          value={settings.max_results}
          onChange={(e) => void save({ max_results: Number(e.currentTarget.value) })}
        />
      </label>

      <div className="settings-note">全局快捷键：{settings.hotkey_label}</div>

      <div className="settings-section">用户 Alias</div>
      <div className="alias-form">
        <input placeholder="别名，如 code" value={alias} onChange={(e) => setAlias(e.target.value)} />
        <input
          placeholder="应用名，如 Visual Studio Code"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
        />
        <button type="button" onClick={() => void addAlias()}>
          添加
        </button>
      </div>
      <ul className="alias-list">
        {aliases.map((a) => (
          <li key={a.alias}>
            <code>{a.alias}</code> → {a.target_name}
            <button type="button" onClick={() => void removeAlias(a.alias)}>
              删除
            </button>
          </li>
        ))}
      </ul>

      <button type="button" className="rescan-btn" onClick={() => void rescan()}>
        重新扫描应用
      </button>
      {msg && <div className="settings-msg">{msg}</div>}
    </div>
  );
}
