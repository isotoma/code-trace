# Delta for emit (trace emission & usage metrics)

## ADDED Requirements

### Requirement: Per-trace current context size metric
The system SHALL emit a `current_context_size` numeric key on each generation observation's `usageDetails` when token usage data is available for the trace's final assistant step.

#### Scenario: OpenCode turn with reasoning-bearing usage
- GIVEN an OpenCode session turn whose last assistant message with `output > 0` carries `tokens` including `reasoning`
- WHEN code-trace builds the Langfuse ingestion batch for that turn
- THEN the generation observation's `usageDetails` SHALL contain `current_context_size` equal to `input + output + reasoning + cache.read + cache.write` of that last assistant message (matching the OpenCode TUI "Context" panel)

#### Scenario: Claude Code turn
- GIVEN a Claude Code transcript turn whose last assistant message with `output_tokens > 0` carries Anthropic `message.usage`
- WHEN code-trace builds the ingestion batch
- THEN `usageDetails.current_context_size` SHALL equal `input_tokens + output_tokens + cache_read_input_tokens + cache_creation_input_tokens` of that last assistant message (`reasoning_tokens` absent → 0)

#### Scenario: Multi-step tool-call turn uses the last step, not the sum
- GIVEN a turn with multiple assistant messages each carrying usage with `output > 0`
- WHEN the batch is built
- THEN `current_context_size` SHALL reflect only the last qualifying assistant message, and SHALL NOT equal the summed per-step usage

#### Scenario: Last assistant step lacks usage, earlier step has it
- GIVEN a turn whose final assistant message has no `usage` block but an earlier assistant message has usage with `output > 0`
- WHEN the batch is built
- THEN `current_context_size` SHALL fall back to that earlier qualifying message's value

## ADDED Requirements

### Requirement: Omit metric when data is absent
The system SHALL NOT emit `current_context_size` when no qualifying assistant message exists, and SHALL NOT emit it as `0`.

#### Scenario: Turn with no usage data at all
- GIVEN a turn whose assistant messages carry no `usage` blocks
- WHEN the batch is built
- THEN no `usageDetails` object SHALL be created and no `current_context_size` key SHALL appear anywhere

#### Scenario: All assistant steps have zero output
- GIVEN a turn whose only assistant messages with usage have `output_tokens == 0`
- WHEN the batch is built
- THEN `usageDetails` MAY be present (from summed usage) but SHALL NOT contain `current_context_size`

## MODIFIED Requirements

### Requirement: Existing usage keys are unchanged
The system SHALL preserve the exact values of the existing `usageDetails` keys (`input`, `output`, `cache_creation_input_tokens`, `cache_read_input_tokens`) and their per-turn summed computation. `current_context_size` is additive only.

#### Scenario: Regression — four existing keys
- GIVEN any turn that produced `usageDetails` before this change
- WHEN the batch is built after this change
- THEN the four existing keys SHALL have byte-for-byte identical values to before, and `reasoning_tokens` SHALL NOT appear as a `usageDetails` key

## ADDED Requirements

### Requirement: Pi traces unaffected
The system SHALL NOT emit `current_context_size` on traces produced from Pi agent sessions, since Pi normalised messages carry no `usage` block.

#### Scenario: Pi session
- GIVEN a Pi agent session turn
- WHEN the batch is built
- THEN the generation observation SHALL have no `usageDetails` object and no `current_context_size` key
