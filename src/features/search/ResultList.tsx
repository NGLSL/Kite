import type { RefObject } from "react";
import type { SearchResult } from "../../types/ipc";
import { ResultItem } from "./ResultItem";

type Props = {
  results: SearchResult[];
  active: number;
  query: string;
  listRef: RefObject<HTMLDivElement | null>;
  onSelect: (index: number) => void;
  onLaunch: (item: SearchResult) => void;
  /** 结果容器滚动到底部时触发（滚动加载）。 */
  onScroll?: (e: React.UIEvent<HTMLDivElement>) => void;
};

export function ResultList({
  results,
  active,
  query,
  listRef,
  onSelect,
  onLaunch,
  onScroll,
}: Props) {
  return (
    <div className="results" ref={listRef} role="listbox" onScroll={onScroll}>
      {results.map((item, i) => (
        <ResultItem
          key={item.id}
          item={item}
          index={i}
          active={i === active}
          query={query}
          onSelect={onSelect}
          onLaunch={onLaunch}
        />
      ))}
    </div>
  );
}
