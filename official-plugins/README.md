# 官方插件

官方插件用 **Rust** 实现（性能与启动开销考虑），独立进程，stdio JSON-RPC；**协议不限制第三方语言**。

## 布局

```text
official-plugins/          # 独立 Cargo workspace（不进入 kite 主 crate）
  kite-plugin-sdk/         # Content-Length + JSON-RPC 最小 SDK（Rust 作者可选用）
  calculator/              # =  → Panel / copy_text
  window-switcher/         # win → List / plugin_action activate_window
  devtools/                # uuid|hash|ts|json → 多 Provider Panel
```

每个插件 crate 目录含 `plugin.json`（`runtime.command` 直接指向 exe，无 run.cmd/Python 回退）。

构建后打包到：

```text
resources/official-plugins/<plugin-id>/
  plugin.json
  kite-plugin-*.exe
```

用户侧目录：`%APPDATA%\com.kite.launcher\plugins\`。

**内置方式**：安装包携带 `resources/official-plugins`；Kite **首次启动**自动把缺失的官方插件拷入用户目录（不覆盖已有 id）。设置 → 插件 →「安装官方示例」可强制重装。

## 构建与打包

```powershell
.\scripts\build-official-plugins.ps1
```

## 开发说明

语言无关协议见 [`docs/PLUGIN-DEVELOPMENT.md`](../../docs/PLUGIN-DEVELOPMENT.md)。
