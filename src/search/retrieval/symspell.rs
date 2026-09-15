//! SymSpell 风格删除索引：从已索引真实词元生成，不导入外部大词典。

use std::collections::{HashMap, HashSet};

/// 删除变体 → 原词元列表。
#[derive(Debug, Default)]
pub struct DeleteIndex {
    pub(crate) map: HashMap<String, Vec<String>>,
    max_deletes: usize,
}

impl DeleteIndex {
    pub fn lookup(&self, variant: &str) -> Option<&[String]> {
        self.map.get(variant).map(|v| v.as_slice())
    }

    pub fn stats(&self) -> (usize, usize) {
        (
            self.map.len(),
            self.map.values().map(|v| v.len()).sum(),
        )
    }

    /// Query 词的删除变体（含自身）在索引中找到的原词元。
    pub fn expand(&self, word: &str, max_distance: usize) -> Vec<(String, usize)> {
        let mut found: HashMap<String, usize> = HashMap::new();
        let mut seen_variants: HashSet<String> = HashSet::new();
        let mut queue: Vec<(String, usize)> = vec![(word.to_string(), 0)];
        seen_variants.insert(word.to_string());

        while let Some((variant, dist)) = queue.pop() {
            if let Some(terms) = self.lookup(&variant) {
                for t in terms {
                    let d = dist.min(max_distance);
                    let e = found.entry(t.clone()).or_insert(usize::MAX);
                    if d < *e {
                        *e = d;
                    }
                }
            }
            if dist >= self.max_deletes || dist >= max_distance {
                continue;
            }
            for del in deletes_once(&variant) {
                if seen_variants.insert(del.clone()) {
                    queue.push((del, dist + 1));
                }
            }
        }
        found.into_iter().map(|(t, d)| (t, d)).collect()
    }
}

/// 从词元集合构建删除索引。控制长词成本：词长 > 10 只做距离 1，> 18 不建 deletes。
pub fn build(terms: &HashSet<String>, max_deletes: usize) -> DeleteIndex {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for term in terms {
        let len = term.chars().count();
        if len < 2 || len > 18 {
            continue;
        }
        let limit = if len > 10 {
            1
        } else {
            max_deletes
        };
        let mut variants: HashSet<String> = HashSet::new();
        variants.insert(term.clone());
        let mut frontier = vec![term.clone()];
        for _ in 0..limit {
            let mut next = Vec::new();
            for v in &frontier {
                for d in deletes_once(v) {
                    if variants.insert(d.clone()) {
                        next.push(d);
                    }
                }
            }
            frontier = next;
        }
        for v in variants {
            if v == *term {
                continue;
            }
            map.entry(v).or_default().push(term.clone());
        }
    }
    for list in map.values_mut() {
        list.sort();
        list.dedup();
    }
    DeleteIndex { map, max_deletes }
}

fn deletes_once(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 1 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chars.len());
    for i in 0..chars.len() {
        let mut t = String::with_capacity(s.len());
        for (j, c) in chars.iter().enumerate() {
            if j != i {
                t.push(*c);
            }
        }
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_deletion_typo() {
        let mut terms = HashSet::new();
        terms.insert("chrome".into());
        let idx = build(&terms, 2);
        let hits = idx.expand("chorme", 2);
        assert!(
            hits.iter().any(|(t, d)| t == "chrome" && *d <= 1),
            "{hits:?}"
        );
        let hits = idx.expand("crome", 1);
        assert!(hits.iter().any(|(t, _)| t == "chrome"), "{hits:?}");
    }

    #[test]
    fn distance_limited() {
        let mut terms = HashSet::new();
        terms.insert("chrome".into());
        let idx = build(&terms, 2);
        // 距离 3 的删除不应命中
        assert!(idx.expand("chr", 1).is_empty());
    }
}
