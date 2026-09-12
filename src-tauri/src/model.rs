use serde::Serialize;

/// 可启动的 Windows 应用条目（开始菜单 / 桌面 / App Paths）。
#[derive(Debug, Clone, Serialize)]
pub struct AppItem {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// 已缓存 PNG 图标的绝对路径；提取失败为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub source: String,
}

/// 返回给前端的一条排序后的搜索结果。
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub item: AppItem,
    pub score: i32,
    pub matched_by: String,
}

/// 内存中的应用索引；启动 / 重扫时整体重建。
#[derive(Debug, Default)]
pub struct AppIndex {
    pub apps: Vec<AppItem>,
}

impl AppIndex {
    pub fn empty() -> Self {
        Self { apps: Vec::new() }
    }
}
