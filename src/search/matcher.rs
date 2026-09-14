//! 用户 Alias 目标类型。

/// 用户 Alias 的一行匹配素材：优先按稳定 id 命中，旧行（无 id）退回名称包含。
pub struct UserTarget {
    pub id: Option<String>,
    pub name: String,
}
