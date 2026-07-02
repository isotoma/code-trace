# Proposal: fix-paused-turn-replay

## Why

`pause` currently *defers* tracing instead of preventing it: the suppression exit happens before the transcript is read, so the session's cursor never advances while paused, and the first emit after `resume` replays every paused-period turn to Langfuse — exactly the content the user paused to protect. The Track 2 round-trip test pins this behaviour (`[1, 2, 3]` after resume) with a comment flagging it. OpenCode and Pi are not affected (their plugins offer each message once and the suppressed exit discards it), so Claude Code's replay is also an inconsistency between sources.

## What Changes

- **Suppressed invocations consume their input without emitting.** On the emit path, a suppressed session still reads new transcript content (Claude Code) or accepts the piped messages (OpenCode/Pi), advances its cursor (offset, buffer, turn count), saves state, and exits — but never builds events and never sends. Turns that occur while paused become permanently untraceable, for every source.
- Turn numbering in Langfuse gains visible gaps across a pause (e.g. turn 1, then turn 5 after resume) — deliberate: the gap reflects reality.
- Test updates: the round-trip test flips to assert `[1, 3]`; `suppressed_session_emits_nothing_and_advances_no_cursor` (which pins the old no-advance behaviour) inverts to assert the cursor *does* advance while nothing is sent.
- **BREAKING** (semantics, not API): paused-period turns can no longer be recovered by resuming; that is the point. `purge` remains the remediation for anything sent before a pause.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `private-mode`: the "Suppressed sessions are never emitted" requirement changes from "exit before reading any transcript content, no cursor advance" to "consume input and advance the cursor, but never build or send events". Note: `add-privacy-controls` (which ADDs this requirement) must be archived before this change so the delta applies to an existing main spec.

## Impact

- **Code**: `src/main.rs` only — the suppressed early-exit moves from before the source match into each arm, after turn building and cursor update, before event building.
- **Tests**: `tests/concurrency_test.rs` round-trip assertion, `tests/cli_test.rs` suppressed-session test; stress test unaffected (asserts no post-pause *events*, which still holds).
- **Behaviour**: paused sessions do slightly more work per hook (transcript read); nothing is sent, which is the guarantee.
