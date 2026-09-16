use serde::Serialize;

/// 可启动的 Windows 应用条目（正式入口 / 系统入口 / Tier C 发现与命令）。
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
    /// 扫描时记录：是否为已解析的 .lnk（主入口优先于同源裸 exe）。
    #[serde(default)]
    pub is_lnk: bool,
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
    /// 系统入口等附加搜索关键词（小写），随快照入索引。
    #[serde(skip)]
    pub search_keywords: Vec<String>,
    /// 系统提供的扩展搜索词；召回时参与索引，但评分低于明确名称和可信别名。
    #[serde(skip)]
    pub search_context: Vec<String>,
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
            is_lnk: false,
            normalized_name: String::new(),
            normalized_display: String::new(),
            pinyin: String::new(),
            pinyin_initials: String::new(),
            search_keywords: Vec::new(),
            search_context: Vec::new(),
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
    /// 基础相关性层级（数值越小质量越高）。加分前计算；0 表示未标注。
    /// 最终排序以本字段为第一键，禁止用加分后的 score 反推层级。
    #[serde(skip)]
    pub quality_tier: i32,
}

impl SearchResult {
    /// 用基础 MatchScore 构造，并写入显式质量层。
    pub fn scored(item: AppItem, score: i32, matched_by: impl Into<String>) -> Self {
        Self {
            quality_tier: crate::history::quality_tier(score),
            item,
            score,
            matched_by: matched_by.into(),
        }
    }

    /// 未走 MatchScore 通道的结果（文件、网页等）；层按 score 回退。
    pub fn with_score(item: AppItem, score: i32, matched_by: impl Into<String>) -> Self {
        Self::scored(item, score, matched_by)
    }
}

/// 内存中的应用索引；启动 / 重扫时整体重建。
#[derive(Debug, Default)]
pub struct AppIndex {
    pub apps: Vec<AppItem>,
    /// 快照同代的系统入口（Kite 设置 / Windows 设置页 / 系统工具）。
    pub system_entries: Vec<AppItem>,
    /// 与 apps + system_entries 同代的只读检索索引。
    pub retrieval: Option<std::sync::Arc<crate::search::RetrievalIndex>>,
}

impl AppIndex {
    pub fn empty() -> Self {
        Self {
            apps: Vec::new(),
            system_entries: Vec::new(),
            retrieval: None,
        }
    }

    /// 用当前 apps + system_entries 重建检索索引并发布。
    pub fn rebuild_retrieval(&mut self) {
        let index = crate::search::RetrievalIndex::build(&self.apps, &self.system_entries);
        self.retrieval = Some(std::sync::Arc::new(index));
    }
}

/// 索引来源的产品分层（Tier）：用于空 Query 可见性与结果降噪，不表示删除索引。
///
/// - **Tier A Formal**：Start Menu / Desktop / UWP / 用户 Portable
/// - **Tier B System**：Kite curated 系统入口
/// - **Tier C**：`Supplemental`（App Paths / Uninstall）与 `CommandAlias`（包管理器命令）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLayer {
    /// Tier A：用户可识别的正式应用入口。
    Formal,
    /// Tier C：发现来源——可作 alias/元数据/兜底，默认不当正式应用刷屏。
    Supplemental,
    /// Tier C：命令 Alias：Scoop / WindowsApps / WinGet / Chocolatey。
    CommandAlias,
    /// Tier B：Kite curated 系统入口。
    System,
}

/// 由扫描 source 字符串映射到产品分层。**Formal 是白名单**，未知来源默认 Supplemental，
/// 避免未来新扫描器忘记分类就变成正式应用。
pub fn source_layer(source: &str) -> SourceLayer {
    match source {
        "start-menu" | "desktop" | "uwp" | "apps-folder" | "portable" => SourceLayer::Formal,
        "builtin" | "builtin-system" | "win-settings" => SourceLayer::System,
        "app-paths" | "uninstall" => SourceLayer::Supplemental,
        "commands" | "scoop" => SourceLayer::CommandAlias,
        _ => SourceLayer::Supplemental,
    }
}

/// Discovery 来源（App Paths / Uninstall）：可作吸收/兜底，不单独刷屏。
/// 与 `source_layer` 的 Supplemental 白名单对齐；未知 source 不算 Discovery。
pub fn is_discovery_source(source: &str) -> bool {
    matches!(source, "app-paths" | "uninstall")
}

/// 路径型正式入口：开始菜单 / 桌面 / 用户 Portable（不含 UWP）。
pub fn is_path_formal_source(source: &str) -> bool {
    matches!(source, "start-menu" | "desktop" | "portable")
}

/// 空 Query 默认列表是否隐藏该来源（Pin/最近使用仍可覆盖）。
pub fn is_hidden_on_empty_query(source: &str) -> bool {
    matches!(
        source_layer(source),
        SourceLayer::CommandAlias | SourceLayer::Supplemental
    )
}

/// 目标是否落在系统目录（System32/SysWOW64/WindowsApps…）。
/// 空 Query 不把这类正式快捷方式当应用补满；仍可搜索。
pub fn is_system_dir_target(target: &str) -> bool {
    let norm = target.trim().replace('/', "\\").to_ascii_lowercase();
    const FRAGMENTS: &[&str] = &[
        r"\windows\system32\",
        r"\windows\syswow64\",
        r"\windows\winsxs\",
        r"\windowsapps\",
        r"\winget\links\",
        r"\chocolatey\bin\",
        r"\microsoft\windowsapps\",
    ];
    let padded = format!("\\{norm}\\");
    FRAGMENTS.iter().any(|fragment| padded.contains(fragment))
}

/// 空 Query 默认补满是否隐藏（来源层 + 系统目录噪声）。Pin/最近仍可覆盖。
pub fn is_hidden_on_empty_fill(source: &str, target: &str) -> bool {
    is_hidden_on_empty_query(source) || is_system_dir_target(target)
}
