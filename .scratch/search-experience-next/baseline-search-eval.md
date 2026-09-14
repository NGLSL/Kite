# Search eval baseline (ticket 06)

Run: `cargo test search_eval -- --nocapture`

Fixture: `tests/search_cases.json` (`default`), 22 cases.

Recorded on branch `search-experience-next` after ticket 03 lock-in and `wt` alias specificity fix (`terminal` → `windows terminal`).

## All cases

| metric | value |
|---|---|
| cases | 22 |
| Top1 Accuracy | 0.773 |
| Recall@5 | 0.773 |
| MRR | 0.773 |
| latency µs p50 | ~400 |
| latency µs p95 | ~980 |

## Required only (CI gate)

| metric | value |
|---|---|
| cases | 15 |
| Top1 Accuracy | 1.000 |
| Recall@5 | 1.000 |
| MRR | 1.000 |

## Known gaps (not CI-blocking until target tickets land)

| case | query | expected | ticket |
|---|---|---|---|
| todo-compact-gap | `todo` | Microsoft To Do | 07 |
| visual-code-token-gap | `visual code` | Visual Studio Code | 07 |
| vs-code-token-gap | `vs code` | Visual Studio Code | 07 |
| ter-word-prefix-gap | `ter` | XTerminal in top5 (top may be Windows Terminal today) | 07 |
| ndm-acronym-gap | `ndm` | Neat Download Manager | 08 |
| leishe-crosslang-gap | `雷蛇` | Razer Synapse | 08 |
| history-ter-boosts-similar-tier | `ter` + history | XTerminal top | 09 |

## Notes

- Duplicate-entry count is asserted implicitly via fixture uniqueness (distinct args PowerShell retained; friendly vs raw WPS both searchable, raw not first).
- Index coverage is a separate real-machine denominator; this harness only scores matcher/ranker on a fixed fixture.
- Flip `status` from `known-gap` to `required` when the corresponding ticket lands.
