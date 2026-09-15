# 02: 短输入与混输召回覆盖

**What to build:** 短输入与混输 Query 在各搜索字段上有一致覆盖：1 字符可命中名称内部、英文词首、拼音首字母任意位置且不启用纠错；2–3 字符支持连续片段与有依据的缩写，非连续匹配须经顺序、重复次数与间隔验证；较长输入保留多词、混拼音、跳字和纠错。前缀召回不因与相关性无关的固定枚举上限漏掉正确词元。拼音/汉字/英文可共同参与解释，不再被「纯 ASCII Query」总开关一刀切。用户用很短或混输输入时，正确 AppItem 能进入 Top K，而不是只在完整前缀时出现。

**Blocked by:** 01 统一最终排序管线

**Status:** done

- [x] 1 字符：名称内部字符、英文词首、拼音首字母任意位置可召回；不启用拼写纠错
- [x] 2–3 字符：连续片段与词首/首字母类缩写可召回；乱序或超距非连续不得成为高分假命中
- [x] 前缀词元枚举取消与相关性无关的固定小上限，或完整分批扩展；不依赖其他通道碰巧补回
- [x] 拼音候选生成与验证同步支持混输解释，不能只删一个 ASCII 判断
- [x] 固定样本覆盖：单字符内部命中、短片段、短缩写、拼音首字母中段、汉字+拼音/英文混输；期望指向稳定 fixture id
- [x] 扩大召回后常见短 Query 延迟仍可用（记录 P50/P95，不明显恶化）；`cargo test` 全绿，既有 required 样本不回退

## Comments

2026-03-12 实现记录：

- `MAX_PREFIX_TERMS` 16 → 512；1–2 字符靠 char/first_char/gram 收窄。
- 1 字符：名称内部 `char_postings`；拼音首字母 `contains` 任意位置；禁用 SymSpell/fuzzy。
- 混输：`ParsedQuery.mixed/cjk_parts/latin_parts`；通道按段锚定；验证 `mixed-cjk-latin`（CJK 段含名称 + 拼音/词元覆盖拉丁段）。
- 拼音通道由 `has_ascii_alnum` 门控，不再要求纯 ASCII。
- ib-pinyin matcher 对混输启用。
- search_cases 新增：`n`、`z`、`pad`、`nd`、`微信xin`、`w微`。
- eval：短 Query P50 ~10ms 量级；required 全过。
- `cargo test` 284 passed。
