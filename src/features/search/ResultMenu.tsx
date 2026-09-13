/** 搜索结果上下文菜单：鼠标右键 / Menu 键呼出，键盘可操作，Esc 与点击外部关闭。 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { SearchResult } from "../../types/ipc";

/** 非文件系统目标前缀（与 Rust actions 模块一致；Rust 仍是最终裁决）。 */
const NON_FS_PREFIXES = ["shell:", "ms-settings:", "ms-clock:", "ms-contact-support:", "kite:"];

export type MenuAction = "open_folder" | "copy_path" | "copy_name" | "pin" | "unpin";

/** 结果是否指向真实文件系统路径（决定「打开所在文件夹」是否展示）。 */
export function hasFsTarget(target: string | undefined | null): boolean {
  const t = (target ?? "").trim();
  if (!t || NON_FS_PREFIXES.some((p) => t.startsWith(p))) return false;
  return !(t.startsWith("http://") || t.startsWith("https://"));
}

type Props = {
  item: SearchResult;
  x: number;
  y: number;
  pinned: boolean;
  onAction: (a: MenuAction) => void;
  onClose: () => void;
};

export function ResultMenu({ item, x, y, pinned, onAction, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  const [hi, setHi] = useState(0);

  const entries: { action: MenuAction; label: string }[] = [];
  if (hasFsTarget(item.target)) {
    entries.push({ action: "open_folder", label: "打开所在文件夹" });
  }
  const t = (item.target ?? "").trim();
  if (t && !NON_FS_PREFIXES.some((p) => t.startsWith(p))) {
    entries.push({ action: "copy_path", label: "复制路径" });
  }
  entries.push({ action: "copy_name", label: "复制名称" });
  entries.push({ action: pinned ? "unpin" : "pin", label: pinned ? "取消固定" : "固定" });

  // 位置钳制：整个菜单保持在窗口内
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - el.offsetWidth - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - el.offsetHeight - 4)),
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 键盘：↑↓ 选择、Enter 确认、Esc 关闭（capture + stopPropagation，
  // 防止输入框把 Esc 当成「隐藏窗口」）；Alt+1..9 等不拦截
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        e.stopPropagation();
        setHi((i) => (i + 1) % entries.length);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        setHi((i) => (i - 1 + entries.length) % entries.length);
      } else if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        onAction(entries[hi].action);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hi, onAction, onClose]);

  // 点击菜单外部关闭
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onDown, true);
    return () => window.removeEventListener("mousedown", onDown, true);
  }, [onClose]);

  return (
    <div className="ctx-menu" ref={ref} style={{ left: pos.left, top: pos.top }} role="menu">
      {entries.map((en, i) => (
        <button
          key={en.action}
          type="button"
          role="menuitem"
          className={`ctx-item ${i === hi ? "hi" : ""}`}
          onMouseEnter={() => setHi(i)}
          onClick={() => onAction(en.action)}
        >
          {en.label}
        </button>
      ))}
    </div>
  );
}
