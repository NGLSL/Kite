import { useState } from "react";
import type { SearchResult } from "../../types/ipc";
import { iconSrc } from "../../shared/iconSrc";
import { highlight } from "../../shared/highlight";

type Props = {
  item: SearchResult;
  index: number;
  active: boolean;
  query: string;
  pinned: boolean;
  onSelect: (index: number) => void;
  onLaunch: (item: SearchResult) => void;
  onContextMenu: (item: SearchResult, x: number, y: number) => void;
};

function secondaryLabel(item: SearchResult): string {
  if (item.source === "browser" || item.source === "websearch" || item.source === "everything") {
    return item.target;
  }
  const t = item.target.replace(/\//g, "\\");
  const i = t.lastIndexOf("\\");
  return i >= 0 ? t.slice(i + 1) : t;
}

export function ResultItem({
  item,
  index,
  active,
  query,
  pinned,
  onSelect,
  onLaunch,
  onContextMenu,
}: Props) {
  const [iconFailed, setIconFailed] = useState(false);
  const src = iconFailed ? undefined : iconSrc(item.icon);
  return (
    <div
      data-index={index}
      role="option"
      aria-selected={active}
      className={`item ${active ? "active" : ""}`}
      onMouseMove={() => onSelect(index)}
      onClick={() => onLaunch(item)}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(item, e.clientX, e.clientY);
      }}
    >
      <div className="item-icon">
        {src ? (
          <img
            src={src}
            alt=""
            width={32}
            height={32}
            draggable={false}
            onError={() => setIconFailed(true)}
          />
        ) : (
          <div className="icon-fallback">{item.name.slice(0, 1).toUpperCase()}</div>
        )}
      </div>
      <div className="item-body">
        <div className="item-name">{highlight(item.display_name || item.name, query)}</div>
        <div className="item-path" title={item.target}>
          {secondaryLabel(item)}
        </div>
      </div>
      <div className="item-hint">
        {pinned && (
          <svg className="pin-flag" viewBox="0 0 24 24" width="12" height="12" aria-label="已固定">
            <path
              d="M12 3l4 4-1 1 2 5-3.5 1.5L12 20l-1.5-5.5L7 13l2-5-1-1z"
              fill="currentColor"
            />
          </svg>
        )}
        {index < 9 ? `Alt+${index + 1}` : ""}
      </div>
    </div>
  );
}
