/** 更新检查：对比 GitHub 最新 Release 与当前版本。纯逻辑，便于复用与核对。 */

export const RELEASES_API = "https://api.github.com/repos/NGLSL/Kite/releases/latest";
export const RELEASES_PAGE = "https://github.com/NGLSL/Kite/releases";

export type UpdateStatus =
  | { kind: "latest" }
  | { kind: "available"; latest: string }
  | { kind: "error"; message: string };

/** 宽松 semver 比较：取 v 前缀与数字段，latest 是否比 current 新。 */
export function isNewerVersion(current: string, latest: string): boolean {
  const nums = (v: string) =>
    v
      .trim()
      .replace(/^v/i, "")
      .split(".")
      .map((n) => Number.parseInt(n, 10) || 0);
  const a = nums(current);
  const b = nums(latest);
  for (let i = 0; i < 3; i++) {
    const diff = (b[i] ?? 0) - (a[i] ?? 0);
    if (diff !== 0) return diff > 0;
  }
  return false;
}

/** 查询 GitHub 最新发布；网络失败/限流归为 error。 */
export async function checkUpdate(currentVersion: string): Promise<UpdateStatus> {
  try {
    const res = await fetch(RELEASES_API, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) {
      return { kind: "error", message: `GitHub 返回 ${res.status}` };
    }
    const data = (await res.json()) as { tag_name?: string };
    const latest = data.tag_name?.trim();
    if (!latest) {
      return { kind: "error", message: "发布数据缺少版本号" };
    }
    if (isNewerVersion(currentVersion, latest)) {
      return { kind: "available", latest };
    }
    return { kind: "latest" };
  } catch (e) {
    return { kind: "error", message: String(e) };
  }
}
