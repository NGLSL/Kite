## Problem Statement

Kite 已经具备应用搜索、历史排序、Alias、Everything 文件搜索、设置和系统集成能力，但部分设置尚未真正影响运行行为，搜索结果也缺少常用的后续操作。Alias 目标依赖自由文本，文件搜索和历史记录缺少持久化与管理入口，导致日常使用仍有额外步骤。

## Solution

分三批完善启动器的日常使用闭环：先修复现有设置、扫描、缓存和历史体验；再增加搜索结果的上下文操作；最后将 Alias 目标改为可从 AppItem 索引中选择和校验。保持搜索逻辑在 Rust、IPC 只返回 Top N，并沿用现有 SQLite 与系统集成边界。

## User Stories

1. As a Kite user, I want the result count setting to control the actual result list, so that my preference is respected.
2. As a Kite user, I want rescanning to run once per request, so that refresh is predictable and lightweight.
3. As a Kite user, I want Alias changes to appear immediately, so that I do not need to restart or repeat a query.
4. As a Kite user, I want Kite to remember whether file search is enabled, so that repeated file searches require fewer steps.
5. As a Kite user, I want to clear or pause usage history, so that I can control personalization and privacy.
6. As a Kite user, I want to open an item's containing folder, so that I can locate the executable or file quickly.
7. As a Kite user, I want to copy an item's path or name, so that I can reuse it in another application.
8. As a Kite user, I want to pin a frequently used result, so that it remains easy to reach.
9. As a Kite user, I want to choose an indexed application when creating an Alias, so that I do not save an invalid target name.
10. As a Kite user, I want invalid Alias targets to be rejected, so that saved shortcuts always resolve to an AppItem.

## Implementation Decisions

- Keep the existing Rust search, ranking, history, scanner, launcher, and SQLite boundaries.
- Keep the parent feature split into three sequential implementation issues to avoid concurrent edits to shared IPC code.
- Use explicit result actions identified by item id and action kind; resolve the real target from the index before performing an operation.
- Keep file search opt-in by default while persisting the user's choice in settings.
- Keep history ranking available, while adding explicit recording control and cleanup operations.
- Prefer stable AppItem identity for Alias targets when schema changes are needed; preserve existing Alias data through a migration or safe compatibility path.
- Do not introduce plugins, AI, OCR, cloud sync, cross-platform support, self-built full-disk indexing, or shell execution from raw user input.

## Testing Decisions

- Preserve the existing Rust search regression suite and add tests for changed external behavior.
- Test settings round trips, cache invalidation, history recording controls, Alias validation, and action dispatch at the Rust boundary.
- Test the React behavior through the existing build/typecheck path and targeted interaction checks where practical.
- Run `cd src-tauri; cargo test` and `npm run build` for the completed feature set.
- Perform Windows smoke checks for rescanning, file mode persistence, history controls, path actions, and UWP behavior.

## Out of Scope

- Plugin runtime or marketplace.
- AI, OCR, screenshot, cloud synchronization, accounts, and cross-platform support.
- Full clipboard history.
- A general command shell or arbitrary command execution.
- Replacing Everything with a self-built full-disk index.
- Large-scale theme or animation work.

## Further Notes

- Implementation issue 01 should land before issue 02 because both may touch shared IPC paths.
- Issue 03 can begin after the cache and settings contracts from issue 01 are stable.
- The existing `package-lock.json` worktree change is unrelated and must remain outside these tasks.
