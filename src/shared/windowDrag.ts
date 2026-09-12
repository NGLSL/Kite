/** 无边框窗口拖拽：在标题/空白区按住左键移动窗口。 */

import type { MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const INTERACTIVE = "button, input, select, textarea, a, label, [data-no-drag]";

export function startWindowDrag(e: ReactMouseEvent) {
  if (e.button !== 0) return;
  const t = e.target as HTMLElement | null;
  if (t?.closest(INTERACTIVE)) return;
  void getCurrentWindow()
    .startDragging()
    .catch(() => undefined);
}
