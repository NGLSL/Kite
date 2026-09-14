# resources

- `Everything64.dll` — 来自 voidtools 官方 Everything SDK（https://www.voidtools.com/Everything-SDK.zip）。
  用于与运行中的 Everything 通过 IPC 通信查询文件，不会拉起 Everything 主窗口；
  Everything 未运行时查询直接失败、返回空结果。
  再分发依据 voidtools Everything/SDK 的 MIT 许可（https://www.voidtools.com/License.txt）；
  安装包中的完整版权与许可文本见仓库根目录 `THIRD_PARTY_NOTICES.txt`。
- `open.wav` — Kite 窗口唤起时的音效（winmm PlaySound 播放）。
