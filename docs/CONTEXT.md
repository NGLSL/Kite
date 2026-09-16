# Kite 领域词汇

仅记录业务概念，不写实现细节。

| 术语 | 含义 |
|------|------|
| **AppItem** | 索引中的一条可启动应用（有稳定 id / target） |
| **Query** | 用户在搜索框输入的原始字符串；经 Normalizer 后参与匹配 |
| **MatchScore** | 本次 Query 与某 AppItem 的文本匹配质量（Exact/Prefix/…） |
| **Alias** | 缩写到应用的映射；Phase 2 为内置，用户自定义属后续 |
| **Usage / Frequency** | 某 AppItem 被 Kite 启动的次数 |
| **Recency** | 某 AppItem 最近一次被启动的相对时间 |
| **Query History** | 「某 Query 最终选了哪个 AppItem」的配对记忆 |
| **FinalScore** | MatchScore + 各类加分后的最终排序分 |
| **明确匹配保护** | 高质量 Match（如 Name Exact）不得被历史分压到低质量匹配之下 |
| **Pin（固定）** | 用户手动置顶的结果；空 Query 排在最近使用之前，非空 Query 获得固定加分（与历史加分取较大者，不叠加） |
| **Demote（降权）** | 用户对某入口的可撤销负偏好；非保护层扣分后移，不删除索引项、不改启动目标 |
| **匹配位置证据** | 命中在原名称上的 start/span/gaps/edit_cost；不可映射记未知，不当作最优起点 |
| **搜索代际** | Query 代际 + 索引代际；过期结果不得合并进当前列表 |
| **评分诊断** | 命中字段→验证方式→分/偏好标签→归并入口的调试明细，不改变排序 |
| **SourceLayer / Tier** | 索引来源的产品分层，用于空 Query 可见性与弱匹配降噪，**不是**用户 Demote/Alias。**Tier A Formal（白名单）**：Start Menu/Desktop/UWP/apps-folder/用户 Portable。**Tier B System**：Kite curated 系统入口（`system_entries`），不扫 System32。**Tier C Discovery/Command**：App Paths/Uninstall/Scoop/WinGet/WindowsApps/Chocolatey——可作 alias/元数据/精确命令或兜底，**默认不当正式应用刷屏**。未知 source 默认 Supplemental（不当正式应用） |
