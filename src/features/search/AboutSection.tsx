/** 关于：当前版本、更新检查（对比 GitHub 最新 Release）、技术栈。 */

import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RELEASES_PAGE, checkUpdate, type UpdateStatus } from "./update";
import { Row } from "./SettingsUI";

function updateHint(s: UpdateStatus): string {
  switch (s.kind) {
    case "latest":
      return "已是最新版本";
    case "available":
      return `发现新版本 ${s.latest}`;
    case "error":
      return `检查失败：${s.message}`;
  }
}

export function AboutSection() {
  // null = 版本尚未读到；读到之前不允许检查，避免把未知版本误判为旧版
  const [version, setVersion] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  const checkNow = async () => {
    if (!version) return;
    setChecking(true);
    setUpdate(await checkUpdate(version));
    setChecking(false);
  };

  return (
    <div className="flow-card">
      <Row icon="M12 8h.01M11 12h1v4h1" title="Kite" hint="轻量 Windows 启动器">
        <span className="about-ver">{version ? `v${version}` : "…"}</span>
      </Row>
      <Row
        icon="M12 19V5M5 12l7-7 7 7"
        title="检查更新"
        hint={update ? updateHint(update) : "对比 GitHub 最新发布版本"}
      >
        {update?.kind === "available" ? (
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void openUrl(RELEASES_PAGE)}
          >
            查看发布页
          </button>
        ) : (
          <button
            type="button"
            className="btn"
            disabled={!version || checking}
            onClick={() => void checkNow()}
          >
            {checking ? "检查中…" : "检查"}
          </button>
        )}
      </Row>
      <Row icon="M4 7h16M4 12h16M4 17h10" title="技术栈" hint="Tauri 2 · React 19 · Rust · Apache-2.0">
        <span />
      </Row>
    </div>
  );
}
