type Props = {
  scanning: boolean;
  empty: boolean;
  error: string | null;
};

export function StatusRegion({ scanning, empty, error }: Props) {
  return (
    <>
      {scanning && !empty && !error && (
        <div className="empty-state">正在扫描应用…</div>
      )}
      {empty && (
        <div className="empty-state">
          <div className="empty-title">没有找到相关结果</div>
          <div className="empty-sub">试试其他关键词</div>
        </div>
      )}
      {error && <div className="error-banner">{error}</div>}
    </>
  );
}
