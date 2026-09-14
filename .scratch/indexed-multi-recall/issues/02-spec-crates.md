# 02: 按 spec 接入 fst / ib-pinyin / nucleo-matcher / fixedbitset

**What to build:** 搜索内核改用 spec 指定组件：FST 词典做词元精确/前缀枚举；ib-pinyin 做混合全拼/简拼/多音字验证；nucleo-matcher 对非连续候选做对齐评分（不否决其他通道）；fixedbitset 做字符位图。Windows MSVC 可编译，`cargo test --lib` 绿，许可与二进制体积可接受。

**Blocked by:** 01（内核已落地）

**Status:** resolved

## Acceptance

- [x] `fst::Map` 替代 BTreeMap 词典，前缀枚举走 FST stream
- [x] `ib_pinyin::PinyinMatcher` 替代/补充自实现音节混合匹配，覆盖多音字样例
- [x] `nucleo_matcher::Matcher` 为非连续候选提供对齐分；Query 不命中时不得删除其他通道结果
- [x] `fixedbitset::FixedBitSet` 用于 ASCII 字符位图预过滤
- [x] 既有搜索回归 + 新用例通过；`cargo check --lib` 无告警
- [x] 记录依赖许可（体积待 release 构建实测）

## Spec 依据

- Further Notes：fst Map、ib-pinyin、nucleo-matcher、fixedbitset 为组件选型依据
- Implementation Decisions：FST 词典整体生成；ib-pinyin 混合拼音/多音字；nucleo 只补充非连续证据

## Comments

- 2026-01: 已接入并提交 `84e83d7`。
- 许可：fst Unlicense/MIT；ib-pinyin MIT；nucleo-matcher MPL-2.0；fixedbitset MIT OR Apache-2.0。
- Windows MSVC：`cargo check --lib` / `cargo test --lib` 通过（197 tests）。
- 核对缺口：release 二进制体积未实测；FST 值目前存词元序号，尚未直接挂倒排地址。
