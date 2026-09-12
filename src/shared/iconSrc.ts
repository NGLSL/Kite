/** 把图标路径转成 WebView2 可加载的 URL；供各结果 UI 共用。 */

import { convertFileSrc } from "@tauri-apps/api/core";

export function iconSrc(icon: string | null | undefined): string | undefined {
  if (!icon) return undefined;
  if (icon.startsWith("data:")) return icon;
  try {
    return convertFileSrc(icon);
  } catch {
    return undefined;
  }
}
