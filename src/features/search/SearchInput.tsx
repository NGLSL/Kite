import type { RefObject } from "react";

type Props = {
  query: string;
  onQueryChange: (q: string) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  inputRef: RefObject<HTMLInputElement | null>;
  searchFiles: boolean;
  onToggleFiles: () => void;
  onFocusSearch?: () => void;
};

export function SearchInput({
  query,
  onQueryChange,
  onKeyDown,
  inputRef,
  searchFiles,
  onToggleFiles,
}: Props) {
  return (
    <div className="search-row">
      <svg className="search-icon" viewBox="0 0 24 24" aria-hidden>
        <circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" strokeWidth="1.8" />
        <path d="M20 20l-3.5-3.5" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
      </svg>
      <input
        ref={inputRef}
        className="search-input"
        value={query}
        onChange={(e) => onQueryChange(e.currentTarget.value)}
        onKeyDown={onKeyDown}
        placeholder={searchFiles ? "搜应用 + Everything 文件…" : "Search..."}
        spellCheck={false}
        autoComplete="off"
        aria-label="Search applications"
      />
      <button
        type="button"
        className={`files-toggle ${searchFiles ? "on" : ""}`}
        onClick={onToggleFiles}
        title="用 Everything 搜索文件（默认关闭）"
        aria-pressed={searchFiles}
      >
        文件
      </button>
      {query && (
        <button
          className="clear-btn"
          type="button"
          onClick={() => onQueryChange("")}
          aria-label="Clear"
        >
          ×
        </button>
      )}
    </div>
  );
}
