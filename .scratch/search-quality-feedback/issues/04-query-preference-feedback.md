# 04: 查询偏好反馈

**What to build:** 用户纠正过一次排序，下次输入同一 Query 就能感受到变化。在相关性接近（同层或相邻且证据质量接近）的候选里，本 Query 的选择记录优先于全局启动次数；一次选择产生有限倾向，重复选择逐渐稳定顺序。完整名称精确匹配与明确用户 Alias 继续受保护，不因昨天常用或一次误点被抢第一。关闭或清空历史后偏好立即失效。

**Blocked by:** 01 统一最终排序管线, 03 匹配证据细排

**Status:** done

- [x] 同一 Query 下选择某目标后，在相关性接近的候选中前移；无关弱匹配不得因此进入首屏前列
- [x] 本 Query 配对记录优先于全局 Usage；一次选择有限加分，重复选择对数式趋稳，可定义「相关性接近」边界并写入测试期望
- [x] 输入另一软件完整名称时，历史常用软件不能抢第一；用户 Alias 与明确匹配保护仍成立
- [x] 清空或暂停历史后，后续排序立即不再受益于旧加分
- [x] Pin 与历史取较大者、不叠加越过保护界限
- [x] 固定样本覆盖偏好生效与保护场景；`cargo test` 全绿，相对基线 Top1/MRR 不恶化

## Comments

2026-03-12 实现记录：

- 硬保护层 `PROTECTED_TIER_MAX = 1`（用户 Alias / Name Exact）：始终在前，层内 (tier, FinalScore)。
- 其余候选按 FinalScore（base+boost）竞争，不再把 prefix/词首/缩写锁成不可跨越小层；弱 substring+满历史仍低于 word-prefix（既有保护测试保留）。
- 配对分：`30+14*ln(count)`，封顶 110；单次约 30，明显高于单次 Usage；总加分仍 ≤160。
- Usage 封顶降至 45，避免全局高频压过本 Query 配对。
- Pin 与历史取较大者；清空历史由 storage 既有测试覆盖。
- 新增：`query_pair_beats_global_usage_within_open_band`、`one_pair_selection_is_limited_and_repeats_grow_log`、`name_exact_not_stolen_by_popular_open_candidate`、`empty_personalization_matches_base_order`、`pin_and_history_take_max_not_sum`。
- `cargo test` 289 passed；eval required 全过。
