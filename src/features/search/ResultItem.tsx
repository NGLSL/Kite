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

export function ResultItem({ item, index, active, query, onSelect, onLaunch }: Props) {
  const src = iconSrc(item.icon);
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
          <img src={src} alt="" width={32} height={32} draggable={false} />
        ) : (
          <div className="icon-fallback">{item.name.slice(0, 1).toUpperCase()}</div>
        )}
      </div>
      <div className="item-body">
        <div className="item-name">{highlight(item.display_name || item.name, query)}</div>
        <div className="item-path" title={item.target}>
          {item.target}
        </div>
      </div>
      <div className="item-hint">Alt+{index + 1}</div>
    </div>
  );
}
