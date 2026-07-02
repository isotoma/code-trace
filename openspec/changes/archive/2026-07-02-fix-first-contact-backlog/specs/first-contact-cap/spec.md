# first-contact-cap

## ADDED Requirements

### Requirement: First contact emits only the most recent turn
When the emit path processes a session that has no cursor entry in state, it SHALL emit only the most recent built turn, SHALL advance the cursor past all earlier content, and SHALL count the skipped turns in `turn_count` so the emitted turn's number reflects its true position. This SHALL apply identically to all input sources. A log line SHALL record how many pre-contact turns were skipped.

#### Scenario: Pre-existing transcript on first install
- **WHEN** the first Stop hook fires for a session whose transcript already holds turns 1–3 and no cursor exists
- **THEN** only turn 3 is emitted (numbered 3), the cursor points at the end of the transcript, and subsequent hooks emit normally from there

#### Scenario: Genuinely new session is unaffected
- **WHEN** the first Stop hook fires for a brand-new session whose transcript holds exactly one turn
- **THEN** that turn is emitted as turn 1, exactly as before

#### Scenario: Suppressed first contact emits nothing
- **WHEN** the session is both unseen and suppressed
- **THEN** the suppressed consume-path applies: the cursor advances past everything and nothing is emitted
