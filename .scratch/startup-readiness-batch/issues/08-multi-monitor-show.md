Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 08: 多显示器跟随唤起定位

**What to build:** 每次唤起时按当前光标所在 monitor 计算工作区位置：水平居中、垂直约 1/3，再 show/focus。鼠标在哪块屏按快捷键，面板就出现在哪块屏；不再只使用窗口创建时的静态坐标。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] 唤起路径使用 Win32 光标点与 monitor 工作区 API 重新定位
- [ ] 定位在 gain focus/show 之前完成，避免先闪到旧屏
- [ ] 单显示器行为不回归（仍在工作区合理位置）
- [ ] 不依赖 UI 里已有的光标跟踪状态作为定位来源
- [ ] Windows 实机双屏/多屏验收：光标在 B 屏唤起，面板出现在 B 屏
