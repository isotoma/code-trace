# Work Plan: Fix repo tagging for git worktrees and non-standard clone directories

Source: `planflow/changes/fix-repo-tag-worktree/implementation.md`

## Summary

Five tasks, all agent-owned, touching a single file (`src/tags.rs`). The change adds a `repo_from_remote_url` sibling function to the existing `org_from_remote_url`, wires it into `gather_env_tags` with a show-toplevel fallback, and adds unit + integration tests. A final task runs the full test suite and clippy.

## Dependency graph

```
T1 (add repo_from_remote_url)          [agent, S]
├── T2 (add unit tests)                 [agent, S]  depends on T1
└── T3 (restructure gather_env_tags)    [agent, M]  depends on T1
    └── T4 (add integration tests)      [agent, S]  depends on T3
        └── T5 (full test + clippy)     [agent, S]  depends on T1, T2, T3, T4
            ── checkpoint: human review before merge
```

### Parallel streams

- **T1** is the single entry point. Nothing can start until it lands.
- **T2 and T3** are both unblocked after T1. They touch different parts of `src/tags.rs` (T2 adds tests at the bottom of the test module; T3 replaces the repo/org blocks inside `gather_env_tags`). Since `planflow-apply` commits per-task and both edit the same file, the implementor applies them sequentially — but they can be dispatched together and are logically independent.
- **T4** depends on T3 (needs the restructured `gather_env_tags`).
- **T5** depends on everything and is the final verification gate.

### Human checkpoints

- **After T5**: pause for human review of the full diff before merge. This is the only checkpoint — the change is small, test-verified at each step, and carries no client-facing or credentials risk.

## Allocation

All tasks are `agent`-owned. No human-owned tasks in the code pipeline; the release-notes note about historical traces (AC5.1) is a human action at release time, not a code task, and is already documented in the design.

## Task table

| id | title | owner | depends_on | size | checkpoint |
|----|-------|-------|------------|------|------------|
| T1 | Add `repo_from_remote_url` function | agent | [] | S | false |
| T2 | Add `repo_from_remote_url` unit tests | agent | [T1] | S | false |
| T3 | Restructure `gather_env_tags` repo + org blocks | agent | [T1] | M | false |
| T4 | Add `gather_env_tags` integration tests for repo tag | agent | [T3] | S | false |
| T5 | Run full test suite and clippy | agent | [T1, T2, T3, T4] | S | true |
