import csv
import math
import statistics
import sys
from pathlib import Path

# 用法：python scripts/render-performance-svg.py [输入 csv] [输出 svg]
# 默认仍是 2026-09-16 上午那轮的数据与文件，便于复现原曲线。
csv_path = Path(
    sys.argv[1]
    if len(sys.argv) > 1
    else r"D:\Project\Kite\docs\performance-kite-20260916-011716.csv"
)
out_path = Path(
    sys.argv[2] if len(sys.argv) > 2 else r"D:\Project\Kite\docs\performance-100.svg"
)
subtitle = (
    sys.argv[3]
    if len(sys.argv) > 3
    else "Windows 11 26200 · i5-13490F · Release · Alt+Space → 窗口可见 · 2026-09-16"
)
rows = list(csv.DictReader(csv_path.open(encoding="utf-8-sig")))
opens = [float(r["open_ms"]) for r in rows]
priv = [float(r["private_mb"]) for r in rows]
work = [float(r["working_mb"]) for r in rows]


def pct(vals, q):
    s = sorted(vals)
    i = max(0, min(len(s) - 1, math.ceil(q * len(s)) - 1))
    return s[i]


W, H = 1000, 630
left, right = 100, 900
top1, bot1 = 100, 300
top2, bot2 = 355, 550


def ymap_mem(v, vmin, vmax):
    return bot1 - (v - vmin) / (vmax - vmin) * (bot1 - top1)


def ymap_ms(v, vmin, vmax):
    return bot2 - (v - vmin) / (vmax - vmin) * (bot2 - top2)


n = len(rows)
xs = [left + (right - left) * i / max(1, n - 1) for i in range(n)]

all_m = priv + work
mmin = min(all_m) - 2
mmax = max(all_m) + 2
lmax = max(40.0, max(opens) * 1.15)


def poly(ys):
    return " ".join(f"{x:.1f},{y:.1f}" for x, y in zip(xs, ys))


priv_y = [ymap_mem(v, mmin, mmax) for v in priv]
work_y = [ymap_mem(v, mmin, mmax) for v in work]
open_y = [ymap_ms(v, 0, lmax) for v in opens]

mem_ticks = []
v = math.floor(mmin / 10) * 10
while v <= mmax:
    if v >= mmin:
        mem_ticks.append(v)
    v += 10

open_step = 10 if lmax > 50 else 5
open_ticks = list(range(0, int(lmax) + 1, open_step))

parts = []
parts.append(
    f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">'
)
parts.append(f'<rect width="{W}" height="{H}" rx="18" fill="#101827"/>')
parts.append(
    f'<text x="48" y="42" fill="#f8fafc" font-family="Segoe UI" font-size="22" font-weight="600">'
    f"Kite 0.2.8 · 100 次连续唤起性能曲线</text>"
)
parts.append(
    f'<text x="48" y="68" fill="#94a3b8" font-family="Segoe UI" font-size="13">'
    f"{subtitle}</text>"
)
parts.append(
    '<g font-family="Segoe UI" font-size="12" fill="#cbd5e1">'
    '<line x1="600" y1="43" x2="626" y2="43" stroke="#38bdf8" stroke-width="3"/>'
    '<text x="634" y="47">私有内存（MB）</text>'
    '<line x1="740" y1="43" x2="766" y2="43" stroke="#64748b" stroke-width="3"/>'
    '<text x="774" y="47">工作集（MB）</text>'
    "</g>"
)
parts.append(
    f'<text x="100" y="105" fill="#cbd5e1" font-family="Segoe UI" font-size="13" font-weight="600">'
    f"内存</text>"
)
parts.append('<g stroke="#263449" stroke-width="1" fill="#94a3b8" font-family="Segoe UI" font-size="11">')
for t in mem_ticks:
    y = ymap_mem(t, mmin, mmax)
    parts.append(f'<line x1="100" y1="{y:.1f}" x2="900" y2="{y:.1f}"/><text x="78" y="{y + 4:.1f}">{t:.0f}</text>')
parts.append("</g>")
parts.append('<g stroke="#64748b" stroke-width="1.5"><path d="M100 100V300M100 300H900"/></g>')
parts.append(
    f'<polyline points="{poly(work_y)}" fill="none" stroke="#64748b" stroke-width="2" stroke-linejoin="round"/>'
)
parts.append(
    f'<polyline points="{poly(priv_y)}" fill="none" stroke="#38bdf8" stroke-width="2.5" stroke-linejoin="round"/>'
)

parts.append(
    f'<text x="100" y="355" fill="#cbd5e1" font-family="Segoe UI" font-size="13" font-weight="600">'
    f"唤起耗时（ms）</text>"
)
parts.append('<g stroke="#263449" stroke-width="1" fill="#94a3b8" font-family="Segoe UI" font-size="11">')
for t in open_ticks:
    y = ymap_ms(t, 0, lmax)
    parts.append(f'<line x1="100" y1="{y:.1f}" x2="900" y2="{y:.1f}"/><text x="72" y="{y + 4:.1f}">{t}</text>')
parts.append("</g>")
parts.append('<g stroke="#64748b" stroke-width="1.5"><path d="M100 355V550M100 550H900"/></g>')
parts.append(
    f'<polyline points="{poly(open_y)}" fill="none" stroke="#f59e0b" stroke-width="2.5" stroke-linejoin="round"/>'
)

parts.append('<g fill="#64748b" font-family="Segoe UI" font-size="11" text-anchor="middle">')
for i in [0, 24, 49, 74, 99]:
    parts.append(
        f'<line x1="{xs[i]:.1f}" y1="550" x2="{xs[i]:.1f}" y2="556" stroke="#64748b"/>'
        f'<text x="{xs[i]:.1f}" y="578">{i + 1}</text>'
    )
parts.append("</g>")

avg = statistics.mean(opens)
parts.append(
    f'<text x="48" y="610" fill="#94a3b8" font-family="Segoe UI" font-size="12">'
    f"avg {avg:.2f} ms · P50 {pct(opens, 0.5):.2f} · P95 {pct(opens, 0.95):.2f} · "
    f"P99 {pct(opens, 0.99):.2f} · max {max(opens):.2f} · "
    f"private avg {statistics.mean(priv):.2f} MB · working avg {statistics.mean(work):.2f} MB · n={n}</text>"
)
parts.append("</svg>")

out = out_path
out.write_text("\n".join(parts), encoding="utf-8")
print(f"wrote {out}")
print(
    f"open avg={avg:.2f} p50={pct(opens, 0.5):.2f} p95={pct(opens, 0.95):.2f} "
    f"p99={pct(opens, 0.99):.2f} min={min(opens):.2f} max={max(opens):.2f}"
)
print(f"private avg={statistics.mean(priv):.2f} working avg={statistics.mean(work):.2f}")
