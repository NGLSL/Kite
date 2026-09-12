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
