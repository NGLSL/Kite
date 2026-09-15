# 05: 评分诊断明细

**Status:** ready-for-agent
**Progress:** implemented

- [x] `search::diagnose`：`HitDiagnostic`（字段/方式/matched_by/分/层/偏好标签/归并代表）
- [x] `search_personalized_explained` 与无诊断入口排序一致
- [x] preference_tags：pin / history / demote
- [x] 不分叉排序逻辑；单测覆盖标签与顺序不变

## Notes

- 调试产物，默认 UI 不展示；供维护者解释排序
