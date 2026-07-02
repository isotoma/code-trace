# Proposal: fix-first-contact-backlog

## Why

A user reported that freshly installing and enabling code-trace emitted traces from *before* it was installed. Reproduced: when the first Stop hook fires for a session with no cursor entry, the emit path reads the transcript from byte 0 and emits every prior turn. Three triggers share this shape: installing mid-session, `claude --resume` of a pre-install (or cursor-pruned) session, and enabling `TRACE_TO_LANGFUSE` after sessions already ran. It is the sibling of the pause-replay bug: content the user never intended to trace ships because the cursor didn't cover it.

## What Changes

- **First-contact cap**: when a session has no cursor entry, the emit path SHALL emit only the most recent turn and fast-forward the cursor past everything before it, counting the skipped turns in `turn_count` so numbering shows the gap. The turn that fired the hook completed while code-trace was installed and enabled — it is legitimately traceable; everything earlier predates first contact.
- Invisible in the normal case: a genuinely new session's first hook sees a one-turn transcript, so "last turn" = "only turn". No heuristics, timestamps, or configuration.
- Applies to all three sources (a history-sized first payload from an OpenCode/Pi plugin is capped identically).
- Resulting invariant, paired with the pause guarantee: a turn is emitted only if it completed after code-trace first saw its session, and while the session wasn't paused.
- **Documented limitation** (unchanged behaviour): with a cursor already present, turns completed while `TRACE_TO_LANGFUSE=false` still emit on re-enable — the disabled path exits before any state work by design. `pause` is the supported "don't trace this" mechanism; the global flag is an on/off switch.

## Capabilities

### New Capabilities

- `first-contact-cap`: emit-path behaviour for sessions with no cursor entry — cap to the latest turn, fast-forward past the backlog, count skipped turns.

### Modified Capabilities

None.

## Impact

- **Code**: `src/main.rs` — detect cursor absence per arm, skip all but the last built turn.
- **Tests**: new red test (fresh state + multi-turn transcript → only the last turn emits); `langfuse_only_purge_preserves_state_and_does_not_resurrect` currently seeds two turns via a single first emit and must build its history turn-by-turn instead (it depended on the backlog behaviour).
- **Docs**: README note that on first contact only the latest turn is traced.
