use crate::model::{AppItem, SearchResult};

pub const TOP_N: usize = 10;

const SCORE_EXACT: i32 = 1000;
const SCORE_PREFIX: i32 = 800;

fn normalize_query(q: &str) -> String {
    q.trim().to_lowercase()
}

fn normalize_name(n: &str) -> String {
    n.trim().to_lowercase()
}

pub fn search(apps: &[AppItem], query: &str) -> Vec<SearchResult> {
    let q = normalize_query(query);
    if q.is_empty() {
        // Default list: first apps alphabetically (already sorted in index)
        return apps
            .iter()
            .take(TOP_N)
            .map(|item| SearchResult {
                item: item.clone(),
                score: 0,
                matched_by: "default".into(),
            })
            .collect();
    }

    let mut hits: Vec<SearchResult> = Vec::new();
    for item in apps {
        let name = normalize_name(&item.name);
        let display = normalize_name(&item.display_name);

        let mut best: Option<(i32, &str)> = None;
        for candidate in [&name, &display] {
            if candidate == &q {
                let s = SCORE_EXACT;
                best = Some(match best {
                    Some((bs, _)) if bs >= s => (bs, "exact"),
                    _ => (s, "exact"),
                });
            } else if candidate.starts_with(&q) {
                let s = SCORE_PREFIX;
                best = Some(match best {
                    Some((bs, bm)) if bs >= s => (bs, bm),
                    _ => (s, "prefix"),
                });
            }
        }

        if let Some((score, matched_by)) = best {
            hits.push(SearchResult {
                item: item.clone(),
                score,
                matched_by: matched_by.to_string(),
            });
        }
    }

    // Higher score first; then shorter name; then alpha for stability.
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.item.name.len().cmp(&b.item.name.len()))
            .then_with(|| a.item.name.to_lowercase().cmp(&b.item.name.to_lowercase()))
    });
    hits.truncate(TOP_N);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> AppItem {
        AppItem {
            id: name.to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            target: format!("C:\\fake\\{name}.exe"),
            args: None,
            working_dir: None,
            icon: None,
            source: "test".into(),
        }
    }

    #[test]
    fn exact_beats_prefix() {
        let apps = vec![item("Google Chrome"), item("Chrome Remote Desktop")];
        let hits = search(&apps, "chrome");
        assert_eq!(hits[0].name_if(), "Google Chrome".to_string());
    }

    #[test]
    fn prefix_matches() {
        let apps = vec![item("Visual Studio Code"), item("Notepad")];
        let hits = search(&apps, "vis");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched_by, "prefix");
    }

    impl SearchResult {
        fn name_if(&self) -> String {
            self.item.name.clone()
        }
    }
}
