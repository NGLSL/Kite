# Kite

Kite 是一个面向 Windows 的轻量应用启动器。按下快捷键，输入应用名称、别名或拼音，即可快速找到并启动程序。

Kite 使用 Rust 和 Iced 构建原生桌面应用，采用 tiny-skia 软渲染，不依赖 Tauri、WebView2、React 或 Node.js。

<p align="center">
  <img src="docs/Kite-Github.png" alt="Kite" width="440">
</p>

Kite 通过全局快捷键呼出搜索窗口，输入应用名称、别名或拼音后直接启动目标程序。

## 界面预览

<p align="center">
  <img src="docs/Kite-界面截图.png" alt="Kite 搜索窗口" width="638">
</p>

搜索窗口保持轻量布局，左侧显示应用图标和名称，右侧显示快捷启动序号；输入应用名称、别名或拼音即可快速筛选结果。

## 功能

- 全局快捷键唤起和隐藏搜索窗口
- 扫描开始菜单、桌面、Scoop shims、注册表 App Paths 和 Windows packaged apps
- 支持 `.lnk`、`.exe`、UWP 应用、别名、中文、拼音和拼音首字母搜索
- Exact、Prefix、Substring、Fuzzy 多路召回和统一排序
- 启动历史、固定结果、设置和系统托盘
- 应用图标提取与失败降级
- 可选 Everything 文件搜索
- 可搜索回收站、控制面板、注册表编辑器等常用 Windows 系统工具

## 系统要求

- Windows 10/11 x64
- 运行程序无需安装 WebView2 或 Node.js

## 下载与安装

从 GitHub Releases 下载 `kite-setup.exe`，运行安装程序即可。安装器会自动部署程序和运行资源，创建开始菜单与桌面快捷方式，并注册卸载入口。Kite 当前只发布安装版，避免便携运行时遗漏 DLL、音效或注册信息。

当前发布的安装包尚未由 SignPath Foundation 签名。签名申请状态、发布者身份与隐私说明见下方 [Code signing policy](#code-signing-policy)。

## 从源码构建

环境要求：Rust stable、MSVC 工具链和 Visual Studio Build Tools 的 Desktop development with C++ 工作负载。

```powershell
cargo test
cargo build --release
.\scripts\build-installer.ps1
```

构建产物位于：

```text
artifacts/kite-setup.exe
```

## 项目结构

```text
src/          Rust 业务逻辑与 Iced UI
icons/        应用图标
resources/    运行资源
installer/    NSIS 安装脚本
scripts/      构建脚本
docs/         开发文档、需求和 ADR
```

## 性能

在 Windows 11、i5-13490F、32 GB 内存环境下，Iced 原生版本为单进程，稳定空闲私有内存约 14.86 MB，20 秒 CPU 采样无增量，主程序约 7.90 MiB，NSIS 安装包约 3.91 MiB。详细测量方法、100 次唤起曲线和 Flow Launcher 对比见 [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md)。

## 贡献

提交修改前请阅读 [`AGENTS.md`](AGENTS.md) 和 [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)，并至少运行 `cargo test` 和 `cargo build --release`。Issue 和 Pull Request 请说明变更内容、验证方式以及适用的 Windows 环境。

## 开源参考

Kite 的产品定位和工程实践参考了以下开源启动器项目：

- [ZeroLaunch-rs](https://github.com/ghost-him/ZeroLaunch-rs/)
- [LaunchyQt](https://github.com/samsonwang/LaunchyQt)
- [Flow Launcher](https://github.com/Flow-Launcher/Flow.Launcher)

## 相关社区

感谢 [LINUX DO](https://linux.do/) 社区对开源项目的支持。

## Code signing policy

Kite 正准备申请 SignPath.io 的免费开源项目签名，目前尚未获批，现有发布文件也未使用 SignPath Foundation 证书签名。若申请获批且发布文件实际完成签名，署名将为：**Free code signing provided by SignPath.io, certificate by SignPath Foundation.**

项目角色、签名审批流程、当前状态与隐私说明见 [完整 Code signing policy](CODE_SIGNING_POLICY.md)。[下载与发布页面](https://github.com/NGLSL/Kite/releases)会标明各版本的实际签名状态。

## 许可证

本项目使用 [Apache License 2.0](LICENSE) 发布。

安装包内的 Everything SDK DLL 使用 MIT 许可，版权与许可原文见 [第三方声明](THIRD_PARTY_NOTICES.txt)。
