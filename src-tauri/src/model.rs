use serde::Serialize;

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
    /// Absolute path to a cached PNG icon, or None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub item: AppItem,
    pub score: i32,
    pub matched_by: String,
}
