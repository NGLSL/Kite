# 02: 按 spec 接入 fst / ib-pinyin / nucleo-matcher / fixedbitset

**What to build:** 搜索内核改用 spec 指定组件：FST 词典做词元精确/前缀枚举；ib-pinyin 做混合全拼/简拼/多音字验证；nucleo-matcher 对非连续候选做对齐评分（不否决其他通道）；fixedbitset 做字符位图。Windows MSVC 可编译，`cargo test --lib` 绿，许可与二进制体积可接受。

**Blocked by:** 01（内核已落地）

**Status:** ready-for-agent

## Acceptance

- [ ] `fst::Map` 替代 BTreeMap 词典，前缀枚举走 FST stream
- [ ] `ib_pinyin::PinyinMatcher` 替代/补充自实现音节混合匹配，覆盖多音字样例
- [ ] `nucleo_matcher::Matcher` 为非连续候选提供对齐分；Query 不命中时不得删除其他通道结果
- [ ] `fixedbitset::FixedBitSet` 用于 ASCII 字符位图预过滤
- [ ] 既有搜索回归 + 新用例通过；`cargo check --lib` 无告警
- [ ] 记录依赖许可与 release 体积变化

## Spec 依据

- Further Notes：fst Map、ib-pinyin、nucleo-matcher、fixedbitset 为组件选型依据
- Implementation Decisions：FST 词典整体生成；ib-pinyin 混合拼音/多音字；nucleo 只补充非连续证据

## Comments

- 2026-01: 依赖已 `cargo add`，集成进行中（FST 已替换词典，ib-pinyin/nucleo/fixedbitset 待接入验证路径）。
