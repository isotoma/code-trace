# Design: fix-paused-turn-replay

## Context

Chosen in exploration (2026-07-02): option A — advance the cursor on the suppressed path — over fast-forwarding at `resume` time (option B) and documenting the replay (option C). A gives the strong guarantee ("turns that occur while paused are never traced"), enforces it continuously rather than at one moment, and matches OpenCode/Pi semantics where paused-period messages are offered once and gone.

## Goals / Non-Goals

**Goals:**
- A turn occurring while its session is suppressed is never emitted, including after `resume`.
- All three sources behave identically.
- "Nothing was sent" remains provable with exact assertions (sync-send).

**Non-Goals:**
- Recovering paused-period turns (explicitly impossible now — that is the guarantee).
- Changing pause/resume targeting, purge, or the startup reminder.

## Decisions

1. **Consume-in-place, per source arm.** Keep the early `record_session` + suppression *check* where it is, but move the *exit* into each source's match arm: read/normalize input, `build_turns`, advance `offset`/`buffer`/`turn_count`, `touch`, save — then exit 0 before any `build_ingestion_batch` call. The enforcement point stays "before any event is built or send forked"; only the transcript read moves inside the suppressed path (local I/O, sends nothing).
2. **Advance `turn_count` by the number of consumed turns**, so post-resume turn numbers show a gap where the pause was. Alternative — freeze numbering — rejected: dense numbering would silently misrepresent the session timeline.
3. **The original "no transcript bytes are consumed" wording is dropped deliberately.** It was a proxy for the real guarantee (nothing leaves the machine), and keeping it is what causes the replay. Reading locally while suppressed is harmless; emitting later is not.

## Risks / Trade-offs

- [A crash after cursor save but before exit is indistinguishable from the normal path] → no risk: the suppressed path never builds events, so there is nothing in flight to lose or leak.
- [Paused sessions now do transcript I/O per hook] → same cost as normal tracing minus the send; negligible.
- [Users may expect resume to backfill] → README wording states paused turns are never traced.
