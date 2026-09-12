import { useState } from "react";
import type { SearchResult } from "../../types/ipc";
import { iconSrc } from "../../shared/iconSrc";
import { highlight } from "../../shared/highlight";

type Props = {
  item: SearchResult;
  index: number;
  active: boolean;
  query: string;
  onSelect: (index: number) => void;
  onLaunch: (item: SearchResult) => void;
};

function secondaryLabel(item: SearchResult): string {
  if (item.source === "browser" || item.source === "websearch" || item.source === "everything") {
    return item.target;
  }
  const t = item.target.replace(/\//g, "\\");
  const i = t.lastIndexOf("\\");
  return i >= 0 ? t.slice(i + 1) : t;
}

export function ResultItem({ item, index, active, query, onSelect, onLaunch }: Props) {
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
      <div className="item-hint">{index < 9 ? `Alt+${index + 1}` : ""}</div>
    </div>
  );
}
