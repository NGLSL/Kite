/** 别名设置：目标从索引候选中选择（带稳定 id），Rust 保存前校验目标存在。 */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AliasCandidate, UserAlias } from "../../types/ipc";
import { iconSrc } from "../../shared/iconSrc";

type Props = {
  notify: (msg: string) => void;
};

export function AliasSettings({ notify }: Props) {
  const [aliases, setAliases] = useState<UserAlias[]>([]);
  const [alias, setAlias] = useState("");
  const [target, setTarget] = useState("");
  const [targetId, setTargetId] = useState<string | null>(null);
  const [candidates, setCandidates] = useState<AliasCandidate[]>([]);
  const [open, setOpen] = useState(false);
  const debounceRef = useRef<number | undefined>(undefined);
  const boxRef = useRef<HTMLDivElement>(null);

  const reload = useCallback(async () => {
    try {
      setAliases(await invoke<UserAlias[]>("list_user_aliases"));
    } catch (e) {
      notify(String(e));
    }
  }, [notify]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // 输入目标名称 → 索引候选 Top N（IPC 只回 Top N，不传完整索引）
  useEffect(() => {
    window.clearTimeout(debounceRef.current);
    if (!target.trim()) {
      setCandidates([]);
      setOpen(false);
      return;
    }
    debounceRef.current = window.setTimeout(() => {
      void invoke<AliasCandidate[]>("search_alias_targets", { query: target, limit: 8 })
        .then((list) => {
          setCandidates(list);
          setOpen(list.length > 0);
        })
        .catch(() => setCandidates([]));
    }, 120);
    return () => window.clearTimeout(debounceRef.current);
  }, [target]);

  // 点击候选列表外部收起
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (boxRef.current && !boxRef.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const pick = (c: AliasCandidate) => {
    setTargetId(c.id);
    setTarget(c.display_name || c.name);
    setOpen(false);
  };

  const addAlias = async () => {
    if (!alias.trim() || !target.trim()) return;
    if (!targetId) {
      notify("请从候选列表选择目标应用");
      return;
    }
    try {
      const list = await invoke<UserAlias[]>("set_user_alias", {
        alias: alias.trim(),
        targetName: target.trim(),
        targetId,
      });
      setAliases(list);
      setAlias("");
      setTarget("");
      setTargetId(null);
      notify("别名已添加");
    } catch (e) {
      notify(String(e));
    }
  };

  const removeAlias = async (a: string) => {
    try {
      setAliases(await invoke<UserAlias[]>("remove_user_alias", { alias: a }));
    } catch (e) {
      notify(String(e));
    }
  };

  return (
    <div className="flow-card alias-card">
      <div className="flow-pad">
        <p className="set-hint">用短词直达应用，例如 code → Visual Studio Code。目标请从候选中选择。</p>
        <div className="alias-form">
          <input
            placeholder="别名"
            value={alias}
            onChange={(e) => setAlias(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void addAlias()}
          />
          <div className="alias-target-box" ref={boxRef}>
            <input
              placeholder="应用名称"
              value={target}
              onChange={(e) => {
                setTarget(e.target.value);
                setTargetId(null); // 名称改动后需重新选择候选
              }}
              onKeyDown={(e) => e.key === "Enter" && void addAlias()}
            />
            {open && candidates.length > 0 && (
              <ul className="alias-candidates" role="listbox">
                {candidates.map((c) => {
                  const src = iconSrc(c.icon);
                  return (
                    <li key={c.id}>
                      <button type="button" onClick={() => pick(c)}>
                        {src ? (
                          <img src={src} alt="" width={18} height={18} />
                        ) : (
                          <span className="cand-fallback">{c.name.slice(0, 1)}</span>
                        )}
                        <span className="cand-name">{c.display_name || c.name}</span>
                        <span className="cand-src">{c.source}</span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!targetId}
            onClick={() => void addAlias()}
          >
            添加
          </button>
        </div>
        {aliases.length > 0 && (
          <ul className="alias-list">
            {aliases.map((a) => (
              <li key={a.alias}>
                <code>{a.alias}</code>
                <span className="alias-arrow">→</span>
                <span className="alias-target">{a.target_name}</span>
                {!a.target_id && <span className="alias-legacy">旧</span>}
                <button type="button" className="link-btn" onClick={() => void removeAlias(a.alias)}>
                  删除
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
