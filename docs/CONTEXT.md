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
