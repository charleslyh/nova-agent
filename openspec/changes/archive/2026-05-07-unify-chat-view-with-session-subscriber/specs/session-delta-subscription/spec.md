## ADDED Requirements

### Requirement: Sequence-based delta subscription
The transcript store MUST provide a subscription API that emits session delta events after a caller-specified starting sequence, so consumers can resume live state tracking from a deterministic position.

#### Scenario: Subscribe after replay boundary
- **WHEN** a caller loads transcript events and determines `start_seq = last_loaded_seq + 1`
- **THEN** the store MUST deliver only delta events whose sequence is greater than or equal to `start_seq`
- **AND** the caller MUST NOT receive historical events before `start_seq`

### Requirement: Multi-subscriber fan-out with cleanup
The JSONL transcript store implementation MUST support multiple concurrent subscribers and MUST stop tracking a subscriber when delivery to that subscriber fails.

#### Scenario: Independent delivery for multiple subscribers
- **WHEN** two or more subscribers are registered with the same or different starting sequence
- **THEN** each subscriber MUST receive matching future delta events independently
- **AND** one slow or dropped subscriber MUST NOT block delivery to other subscribers

#### Scenario: Remove dropped subscriber on send failure
- **WHEN** the store attempts to publish a delta event through an unbounded channel and send returns an error
- **THEN** the store MUST remove that subscriber from its active subscription set
- **AND** subsequent publish operations MUST NOT attempt delivery to the removed subscriber
