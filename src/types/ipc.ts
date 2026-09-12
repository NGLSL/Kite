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
  max_results: number;
  hotkey: string;
  hotkey_label: string;
};

export type UserAlias = {
  alias: string;
  target_name: string;
};
