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
    /// 图标提取源（lnk icon_location 或 exe 路径），不发给前端。
    #[serde(skip)]
    pub icon_src: Option<String>,
    pub source: String,
    /// 索引时预计算：规范化名称（小写、去首尾空白）。
    #[serde(skip)]
    pub normalized_name: String,
    /// 索引时预计算：规范化显示名（搜索热路径直接用，避免每键重复规范化）。
    #[serde(skip)]
    pub normalized_display: String,
    /// 索引时预计算：全拼（无空格），如 weixinkaifazhegongju。
    #[serde(skip)]
    pub pinyin: String,
    /// 索引时预计算：拼音首字母，如 wxkfzgj。
    #[serde(skip)]
    pub pinyin_initials: String,
}

impl AppItem {
    /// 扫描阶段构造；拼音字段在 `attach_search_fields` 时填充。
    pub fn scanned(
        id: String,
        name: String,
        target: String,
        args: Option<String>,
        working_dir: Option<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            id,
            display_name: name.clone(),
            name,
            target,
            args,
            working_dir,
            icon: None,
            icon_src: None,
            source: source.into(),
            normalized_name: String::new(),
            normalized_display: String::new(),
            pinyin: String::new(),
            pinyin_initials: String::new(),
        }
    }

    /// 在扫描完成后补齐拼音字段（不要在搜索热路径里做转换）。
    pub fn attach_search_fields(&mut self) {
        self.normalized_name = crate::search::normalize_for_index(&self.name);
        self.normalized_display = crate::search::normalize_for_index(&self.display_name);
        let (full, initials) = crate::search::pinyin_of(&self.display_name);
        self.pinyin = full;
        self.pinyin_initials = initials;
    }
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
