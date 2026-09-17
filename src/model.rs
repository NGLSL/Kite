use serde::Serialize;

/// Result 的产品来源。管「这条结果从哪来」，不参与 MatchScore / FinalScore。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResultSource {
    App,
    File,
    Web,
    Builtin,
    Plugin {
        plugin_id: String,
        provider_id: String,
    },
}

/// Result 携带的行为。Result 管展示，Action 管行为。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultAction {
    LaunchApp {
        item_id: String,
    },
    OpenFile {
        path: String,
    },
    OpenUrl {
        url: String,
    },
    CopyText {
        text: String,
    },
    Plugin {
        plugin_id: String,
        action_id: String,
        #[serde(default)]
        payload: serde_json::Value,
    },
}

fn target_is_http_url(target: &str) -> bool {
    let t = target.trim();
    t.starts_with("http://") || t.starts_with("https://")
}

impl ResultSource {
    /// 从 AppItem / matched_by 推导展示来源（expand 阶段启发式，不改变启动路径）。
    pub fn from_item(item: &AppItem, matched_by: &str) -> Self {
        match matched_by {
            "file" => return ResultSource::File,
            "everything-status" => return ResultSource::Builtin,
            "websearch" | "url" => return ResultSource::Web,
            "builtin" | "win-settings" | "builtin-system" => return ResultSource::Builtin,
            _ => {}
        }

        let id = item.id.as_str();
        if id.starts_with("file:") {
            return ResultSource::File;
        }
        if id.starts_with("url:") || id.starts_with("websearch:") {
            return ResultSource::Web;
        }
        if id.starts_with("kite:") {
            return ResultSource::Builtin;
        }

        match item.source.as_str() {
            "everything" => return ResultSource::File,
            "websearch" | "url" | "browser" => return ResultSource::Web,
            "builtin" | "builtin-system" | "win-settings" | "everything-status" => {
                return ResultSource::Builtin
            }
            _ => {}
        }

        if target_is_http_url(&item.target) {
            return ResultSource::Web;
        }
        ResultSource::App
    }
}

impl ResultAction {
    /// expand 阶段默认动作推导；复杂内置项仍可走 LaunchApp（item_id 由 UI/启动层解释）。
    pub fn for_item(item: &AppItem, source: &ResultSource) -> Self {
        match source {
            ResultSource::File => ResultAction::OpenFile {
                path: item.target.clone(),
            },
            ResultSource::Web if target_is_http_url(&item.target) => ResultAction::OpenUrl {
                url: item.target.trim().to_string(),
            },
            // 浏览器/网页搜索 id 含 browser 与 payload，启动仍依赖 item_id 解析。
            ResultSource::Web
            | ResultSource::App
            | ResultSource::Builtin
            | ResultSource::Plugin { .. } => ResultAction::LaunchApp {
                item_id: item.id.clone(),
            },
        }
    }

    /// Plugin 动作构造（V1 后续票使用；expand 阶段仅保证类型可表达）。
    pub fn plugin(
        plugin_id: impl Into<String>,
        action_id: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        ResultAction::Plugin {
            plugin_id: plugin_id.into(),
            action_id: action_id.into(),
            payload,
        }
    }
}

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
///
/// expand：在 AppItem 展示模型旁挂上通用 ResultSource / ResultAction，
/// 供后续插件与统一启动路径使用；本阶段不改变排序与启动行为。
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
    /// Result 来源（App/File/Web/Builtin/Plugin）。
    ///
    /// 与 `AppItem.source`（扫描来源字符串）及 `SourceLayer`（索引产品分层）不同：
    /// 这是结果展示/行为边界。serde 键用 `result_source`，避免与 flatten 的
    /// `AppItem.source` 冲突。
    #[serde(rename = "result_source")]
    pub source: ResultSource,
    /// Result 动作；启动/打开/复制等行为应逐步迁到此字段。
    pub action: ResultAction,
}

impl SearchResult {
    /// 用基础 MatchScore 构造，并写入显式质量层；source/action 由 item 推导。
    pub fn scored(item: AppItem, score: i32, matched_by: impl Into<String>) -> Self {
        let quality_tier = crate::history::quality_tier(score);
        Self::with_quality_tier(item, score, matched_by, quality_tier)
    }

    /// 未走 MatchScore 通道的结果（文件、网页等）；层按 score 回退。
    pub fn with_score(item: AppItem, score: i32, matched_by: impl Into<String>) -> Self {
        Self::scored(item, score, matched_by)
    }

    /// 显式质量层 + 推导 source/action（检索物化 / 诊断路径）。
    pub fn with_quality_tier(
        item: AppItem,
        score: i32,
        matched_by: impl Into<String>,
        quality_tier: i32,
    ) -> Self {
        let matched_by = matched_by.into();
        let source = ResultSource::from_item(&item, &matched_by);
        let action = ResultAction::for_item(&item, &source);
        Self {
            item,
            score,
            matched_by,
            quality_tier,
            source,
            action,
        }
    }

    /// 覆盖 expand 字段（构造后微调 source/action）。
    pub fn with_source_action(mut self, source: ResultSource, action: ResultAction) -> Self {
        self.source = source;
        self.action = action;
        self
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

#[cfg(test)]
mod result_expand_tests {
    use super::*;

    fn app_item(id: &str, target: &str, source: &str) -> AppItem {
        let mut item = AppItem::scanned(
            id.into(),
            format!("name-{id}"),
            target.into(),
            None,
            None,
            source,
        );
        item.attach_search_fields();
        item
    }

    #[test]
    fn app_result_gets_launch_app_action() {
        let item = app_item("app.chrome", r"C:\Program Files\Chrome\chrome.exe", "start-menu");
        let hit = SearchResult::scored(item, 1000, "exact");
        assert_eq!(hit.source, ResultSource::App);
        assert_eq!(
            hit.action,
            ResultAction::LaunchApp {
                item_id: "app.chrome".into()
            }
        );
        // expand 不得改变展示/排序用字段
        assert_eq!(hit.item.id, "app.chrome");
        assert_eq!(hit.score, 1000);
        assert_eq!(hit.matched_by, "exact");
    }

    #[test]
    fn file_result_gets_open_file_action() {
        let item = app_item("file:c:\\a\\b.txt", r"C:\A\B.txt", "everything");
        let hit = SearchResult::scored(item, 400, "file");
        assert_eq!(hit.source, ResultSource::File);
        assert_eq!(
            hit.action,
            ResultAction::OpenFile {
                path: r"C:\A\B.txt".into()
            }
        );
    }

    #[test]
    fn websearch_result_maps_to_web_source() {
        let item = app_item("websearch:default:rust", "https://www.bing.com/search?q=rust", "websearch");
        let hit = SearchResult::scored(item, 880, "websearch");
        assert_eq!(hit.source, ResultSource::Web);
        assert_eq!(
            hit.action,
            ResultAction::OpenUrl {
                url: "https://www.bing.com/search?q=rust".into()
            }
        );
    }

    #[test]
    fn browser_id_encoded_web_hit_keeps_launch_app_action() {
        // id 含 browser payload 时，expand 阶段仍用 LaunchApp，启动层继续 parse id。
        let item = app_item("url:chrome:https://example.com", "C:\\chrome.exe", "browser");
        let hit = SearchResult::scored(item, 940, "url");
        assert_eq!(hit.source, ResultSource::Web);
        assert_eq!(
            hit.action,
            ResultAction::LaunchApp {
                item_id: "url:chrome:https://example.com".into()
            }
        );
    }

    #[test]
    fn builtin_result_maps_to_builtin_source() {
        let item = app_item("kite:settings", "kite:settings", "builtin");
        let hit = SearchResult::scored(item, 930, "builtin");
        assert_eq!(hit.source, ResultSource::Builtin);
        assert_eq!(
            hit.action,
            ResultAction::LaunchApp {
                item_id: "kite:settings".into()
            }
        );
    }

    #[test]
    fn plugin_action_constructor_is_available() {
        let action = ResultAction::plugin(
            "com.kite.calculator",
            "copy",
            serde_json::json!({"text":"3"}),
        );
        match &action {
            ResultAction::Plugin {
                plugin_id,
                action_id,
                ..
            } => {
                assert_eq!(plugin_id, "com.kite.calculator");
                assert_eq!(action_id, "copy");
            }
            other => panic!("expected Plugin action, got {other:?}"),
        }
        let source = ResultSource::Plugin {
            plugin_id: "com.kite.calculator".into(),
            provider_id: "calculate".into(),
        };
        let item = app_item("plugin.calc", "calc", "plugin");
        let hit = SearchResult::scored(item, 0, "plugin").with_source_action(source, action);
        assert!(matches!(hit.source, ResultSource::Plugin { .. }));
        assert!(matches!(hit.action, ResultAction::Plugin { .. }));
    }

    #[test]
    fn with_quality_tier_preserves_tier_and_derives_source() {
        let item = app_item("file:c:\\x.md", r"C:\X.md", "everything");
        let hit = SearchResult::with_quality_tier(item, 400, "file", 7);
        assert_eq!(hit.quality_tier, 7);
        assert_eq!(hit.source, ResultSource::File);
        assert_eq!(
            hit.action,
            ResultAction::OpenFile {
                path: r"C:\X.md".into()
            }
        );
    }

    #[test]
    fn serialized_result_keeps_app_item_scan_source_key_distinct() {
        // flatten 后 AppItem.source 仍表示扫描来源；ResultSource 走 result_source。
        let item = app_item("app.x", r"C:\x.exe", "start-menu");
        let hit = SearchResult::scored(item, 1000, "exact");
        let json = serde_json::to_value(&hit).expect("serialize");
        assert_eq!(json["source"], "start-menu");
        assert_eq!(json["result_source"]["kind"], "app");
        assert_eq!(json["action"]["type"], "launch_app");
        assert_eq!(json["action"]["item_id"], "app.x");
    }
}
