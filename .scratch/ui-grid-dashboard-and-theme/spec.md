# Status: ready-for-agent

## Problem Statement

Kite 当前主界面采用单列垂直长条列表展示搜索结果，在 640px 的窗口宽度下横向大面积留白空旷，纵向仅能展示 6~8 个结果，空间利用率低且视觉较为呆板。当用户未输入任何字符刚唤醒启动器时，缺少一个类似 zTools / Launchpad 的高密度桌面控制中枢，高频应用与官方插件（计算器、时间戳、JSON 格式化、文件搜索等）无法在首屏一览无余地快速直达。输入搜索时，单列列表又容易在多项同名或带版本号的条目下发生信息挤压，且缺乏合理的横向空间分配。同时，目前界面仅支持单一硬编码的浅色方案，颜色三段式灰白割裂，缺乏对深色模式（#0F1115 纯净深空碳黑）以及跟随 Windows 系统自动切换的支持。

## Solution

根据讨论共识，重构 Kite 的界面架构与排布体系，建立统一的高质感双态横向布局与三档皮肤系统：

1. **空 Query 默认态：zTools 网格仪表盘（Grid Dashboard）**
   - 搜索框下方铺设高密度横向图标网格；
   - 包含两排“最近使用”（展示高频 App 以及官方插件如计算器、时间戳、JSON、文件等共 16 项）及一排“已固定”；
   - 鼠标单点击直接激活运行，支持键盘方向键（↑↓←→）光标选中回车。

2. **输入 Query 搜索态：横向双列卡片流（2-Column Cards）**
   - 敲击键盘打字时，网格瞬间收缩，切换为左右对称的横向双列卡片；
   - 左列对应 `Alt 1~4`，右列对应 `Alt 5~8`，完整利用横向宽度；
   - 每张卡片包含 36px 原生大图标、清晰应用名、智能缩略路径和物理微立体键帽徽标；
   - 输入插件触发词（如 `= `）时，整行通栏展开醒目的大字号即时运算卡片。

3. **三档纯正皮肤切换（深色 / 浅色 / 跟随系统）**
   - **深色模式（Dark）**：严格采用 `#0F1115` 纯净深空碳黑底色 + 1px `#2A2D32` 微光细边框 + `#181A20` 悬浮卡片 + `#E5E7EB` 锐利高对比白灰字，杜绝任何发虚发雾的灰蒙蒙材质；
   - **浅色模式（Light）**：纯净象牙白 `#FFFFFF` 底色 + `#E5E7EB` 柔和细边框 + 深灰黑 `#111827` 文字；
   - **跟随系统（System）**：读取 Windows 注册表 `AppsUseLightTheme`，无缝随操作系统日夜模式自动切换。

4. **视觉质感精细化**：
   - 彻底移除图标外围的灰色圆角框（`ICON_BG`），原生应用图标透明直出，消除“框中框”；
   - 快捷键徽标升级为机械键盘物理质感键帽；
   - 去除三段式割裂横线，改用空气留白和自然微阴影。

## User Stories

1. As a Kite user, I want Kite to display a high-density grid dashboard of recently used apps and plugins when the search box is empty, so that I can launch my frequent tools with a single click without remembering plugin triggers.
2. As a Kite user, I want official plugins like Calculator, Timestamp, and JSON tools to appear as tiles in the default grid dashboard, so that plugins are visible and easily discoverable.
3. As a Kite user, I want the launcher to switch smoothly to a 2-column horizontal card layout as soon as I type a search query, so that the 640px window space is fully utilized and results feel balanced.
4. As a Kite user, I want search result cards to display the full app title, truncated target path, and an Alt+N shortcut keycap badge across both columns, so that I can quickly distinguish similar applications.
5. As a Kite user, I want to use Alt+1 to Alt+4 for the left column and Alt+5 to Alt+8 for the right column, so that keyboard muscle memory is intuitive and fast.
6. As a Kite user, I want clear and instant calculation results (like 36px font) when typing formulas, so that I can copy the result directly with Enter.
7. As a Kite user, I want to choose between Dark theme, Light theme, and Follow System in Settings, so that the launcher always matches my visual preference.
8. As a Kite user, I want the dark theme to use a solid, high-contrast #0F1115 carbon background without blurry translucent gray fog, so that text is sharp and comfortable to read.
9. As a Kite user, I want application icons to be rendered cleanly without an ugly gray rectangular background container, so that icons look native and unconstrained.
10. As a Kite user, I want shortcut hints in the bottom bar and list items to look like tactile physical keycaps, so that the UI has a refined modern desktop feel.
11. As a Kite user, I want my selected theme preference to be saved in local settings, so that my appearance choice persists across restarts.

## Implementation Decisions

- Introduce a dedicated theme module that defines `ThemeMode` (Dark, Light, System) and generates semantic `ThemeTokens`.
- Detect Windows system dark/light preference by reading the `AppsUseLightTheme` DWORD from Windows registry (`Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`) with graceful fallback to Dark.
- Redesign the empty-query layout in `search_view`: replace the old 1-column list with a grid dashboard rendering up to 16 recent items (apps + plugins) in 2 rows of 8 columns, followed by pinned items.
- Redesign the active-query layout in `search_view`: replace the old 1-column list with a 2-column card grid, arranging results left-to-right or top-to-bottom across 2 columns.
- Retain the full-width card layout for Provider mode (e.g. calculator instant card with 36px main result).
- Remove `ICON_BG` for items with valid icons; render icons directly with linear interpolation.
- Introduce tactile keycap widget styling in theme tokens for Alt+N and bottom Action Bar.
- Persist `theme_mode` in SQLite settings table and expose a 3-segment switcher (`跟随系统` | `浅色` | `深色`) in General settings.
- Keep zero runtime penalties: use pure Iced 0.14 native containers, canvas, and styles; keep memory baseline at ~15MB.

## Testing Decisions

- Test the theme token contract: verify Dark (#0F1115) and Light (#FFFFFF) produce correct contrast ratios and distinct surfaces.
- Test system theme detection: verify registry reader handles missing keys, light mode (1), and dark mode (0) safely.
- Test settings persistence: verify SQLite schema migration and round-trip read/write for `theme_mode`.
- Test view rendering seams: test `search_view::view` with empty query (grid dashboard), active query (2-column cards), and provider mode (plugin card) across Dark and Light modes to guarantee panic-free construction.

## Out of Scope

- Blurry acrylic or translucent mica glass that compromises contrast or introduces DWM driver artifacts.
- Custom user-defined CSS or external skin download store.
- Re-architecting plugin RPC protocol.

## Further Notes

- The design strictly respects the user's rejection of blurry grey frosted glass and honors the `#0F1115` + `#2A2D32` high-contrast standard from the repo's original design docs.
- The dual-mode layout (zTools Grid for empty state, 2-Column Cards for search state) provides optimal usability for both mouse and keyboard users.
