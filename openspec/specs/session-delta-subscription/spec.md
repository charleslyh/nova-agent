# session-delta-subscription Specification

## Purpose

Sequence-based session event subscription over JSONL transcript storage: one `subscribe(from_seq)` delivers persisted replay then live tail without a separate snapshot API.

## Requirements

### Requirement: Sequence-based subscription with persisted replay

The transcript store MUST provide `subscribe(from_seq)` that delivers all session events with `seq >= from_seq`, first from persistence then from the live tail, without requiring a separate snapshot call.

#### Scenario: Subscribe includes persisted history from from_seq

- **WHEN** a caller subscribes with `from_seq = N` and persisted records exist with `seq >= N`
- **THEN** the store MUST deliver those persisted records in order before or interleaved only after the live tail registration boundary defined by implementation
- **AND** the caller MUST NOT need to call a separate snapshot or load API to obtain history

#### Scenario: Subscribe continues with live tail without duplicates

- **WHEN** persisted history ends at `last_seq` and the store assigns the next append `seq = last_seq + 1`
- **THEN** `subscribe(from_seq)` MUST deliver persisted rows with `from_seq <= seq <= last_seq` exactly once
- **AND** subsequent live appends with `seq > last_seq` MUST be delivered exactly once on the same subscription

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
