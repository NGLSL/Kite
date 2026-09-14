# Search eval baseline (ticket 06)

Run: `cargo test search_eval -- --nocapture`

Fixture: `tests/search_cases.json` (`default`).

## After ticket 07 (compact / token-seq / word-prefix)

| metric | all | required |
|---|---|---|
| Top1 | ~0.91 | 1.00 |
| Recall@5 | ~0.91 | 1.00 |
| MRR | ~0.91 | 1.00 |

Remaining known gaps (ticket 08/09): `ndm` acronym, `雷蛇` cross-lang, history boost for `ter`.

## Initial baseline (after ticket 03 + 06)

| metric | value |
|---|---|
| cases | 22 |
| Top1 Accuracy | 0.773 |
| Recall@5 | 0.773 |
| MRR | 0.773 |
| latency µs p50 | ~400 |
| latency µs p95 | ~980 |

### Required only

| metric | value |
|---|---|
| cases | 15 |
| Top1 Accuracy | 1.000 |
| Recall@5 | 1.000 |
| MRR | 1.000 |

### Known gaps at initial baseline

| case | query | expected | ticket |
|---|---|---|---|
| todo-compact-gap | `todo` | Microsoft To Do | 07 |
| visual-code-token-gap | `visual code` | Visual Studio Code | 07 |
| vs-code-token-gap | `vs code` | Visual Studio Code | 07 |
| ter-word-prefix-gap | `ter` | XTerminal in top5 | 07 |
| ndm-acronym-gap | `ndm` | Neat Download Manager | 08 |
| leishe-crosslang-gap | `雷蛇` | Razer Synapse | 08 |
| history-ter-boosts-similar-tier | `ter` + history | XTerminal top | 09 |
