/** 设置页共用 UI 原语：图标、行卡片、开关。 */

export function NavIcon({ d }: { d: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path d={d} stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function Row({
  icon,
  title,
  hint,
  children,
}: {
  icon?: string;
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flow-row">
      <div className="flow-row-icon">{icon ? <NavIcon d={icon} /> : null}</div>
      <div className="flow-row-text">
        <div className="flow-row-title">{title}</div>
        {hint && <div className="flow-row-hint">{hint}</div>}
      </div>
      <div className="flow-row-ctrl">{children}</div>
    </div>
  );
}

export function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="flow-toggle-wrap">
      <span className="flow-toggle-label">{checked ? "启用" : "禁用"}</span>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        className={`switch ${checked ? "on" : ""}`}
        onClick={() => onChange(!checked)}
      >
        <span className="switch-knob" />
      </button>
    </div>
  );
}
