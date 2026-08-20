# current-context-size — Work Plan

Source: `planflow/changes/current-context-size/implementation.md`

## Summary

Two parallel entry streams converge at the integration tests. All tasks are agent-owned — the work is mechanical, well-specified, and test-backed with `cargo test` as the gate.

**Stream A — Normaliser (opencode.rs):**
1.1 → 1.2 — add `reasoning_tokens` to `extract_opencode_usage`, then unit-test it (v2 + v1).

**Stream B — Metric (transcript.rs + emit.rs):**
2.1 → 2.2 → 2.3 → 2.4 — add `get_context_size`, test it, wire it into `build_ingestion_batch`, test the emit behaviour (last-not-sum, omitted, fallback, key hygiene).

**Convergence — Integration tests:**
3.1 (needs 1.1 + 2.3), 3.2 (needs 2.3), 3.3 (needs 2.3) can run in parallel once their deps land. 3.4 runs `cargo test` as the final gate.

**Checkpoint:** after 3.4 — human reviews the full suite result before merge.

## Parallelism

- 1.1 and 2.1 start immediately and independently.
- 1.2 starts when 1.1 lands; 2.2 starts when 2.1 lands.
- 2.3 starts when 2.1 lands (doesn't need 2.2).
- 2.4 starts when 2.3 lands.
- 3.1, 3.2, 3.3 start when 2.3 lands (3.1 also needs 1.1).
- 3.4 starts when 3.1, 3.2, 3.3 all land.

## Task table

| id | title | owner | depends_on | size | checkpoint |
|----|-------|-------|------------|------|------------|
| 1.1 | Emit reasoning_tokens from extract_opencode_usage | agent | | S | false |
| 1.2 | Unit tests for reasoning_tokens extraction (v2 + v1) | agent | 1.1 | S | false |
| 2.1 | Add get_context_size to transcript.rs | agent | | S | false |
| 2.2 | Unit tests for get_context_size | agent | 2.1 | S | false |
| 2.3 | Emit current_context_size in build_ingestion_batch | agent | 2.1 | M | false |
| 2.4 | Unit tests for emit (last-not-sum, omitted, fallback, key set) | agent | 2.3 | M | false |
| 3.1 | Extend OpenCode integration test (multi-assistant, reasoning) | agent | 1.1, 2.3 | S | false |
| 3.2 | Extend Claude Code integration test | agent | 2.3 | S | false |
| 3.3 | Pi regression assertion | agent | 2.3 | S | false |
| 3.4 | Full cargo test suite green | agent | 3.1, 3.2, 3.3 | S | true |
