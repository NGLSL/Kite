type Props = {
  scanning: boolean;
  empty: boolean;
  error: string | null;
};

export function StatusRegion({ scanning, empty, error }: Props) {
  return (
    <>
      {scanning && (
        <div className="empty-state scanning-hint">正在加载应用索引…</div>
      )}
      {!scanning && empty && (
        <div className="empty-state">
          <div className="empty-title">没有找到相关结果</div>
          <div className="empty-sub">试试其他关键词</div>
        </div>
      )}
      {error && <div className="error-banner">{error}</div>}
    </>
  );
}
