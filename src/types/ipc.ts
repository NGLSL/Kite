/** IPC 类型：字段名与 Rust serde（snake_case）保持一致。 */

export type AppItem = {
  id: string;
  name: string;
  display_name: string;
  target: string;
  args?: string | null;
  working_dir?: string | null;
  icon?: string | null;
  source: string;
};

export type SearchResult = AppItem & {
  score: number;
  matched_by: string;
};

export type Settings = {
  hide_on_blur: boolean;
  autostart: boolean;
  hotkey: string;
  hotkey_label: string;
  /** 搜索时是否带 Everything 文件结果；默认关闭 */
  search_files: boolean;
  /** 是否记录启动历史（次数 + Query 配对） */
  history_recording: boolean;
};

export type UserAlias = {
  alias: string;
  target_name: string;
  /** 稳定目标标识；旧数据可能缺省 */
  target_id?: string | null;
};

/** Alias 目标选择器的候选（索引子集，Top N） */
export type AliasCandidate = {
  id: string;
  name: string;
  display_name: string;
  icon?: string | null;
  source: string;
};

/** 结果上下文动作（复制类在前端完成，其余走 Rust） */
export type ResultActionKind = "open_folder" | "pin" | "unpin";
