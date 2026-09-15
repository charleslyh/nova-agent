# nova-extensions Specification

## Purpose

Defines **`nova-extensions`**: **extension implementations** of traits from `nova-core`, plus small **domain-generic** helpers that compose those implementations. Extensions MUST NOT encode a specific product's TOML layout, env-var CLI conventions, or a full application harness strategy.

## Requirements

### Requirement: Extensions package layout

The repository SHALL provide `nova-extensions` at `crates/extensions` depending on `nova-core`.

#### Scenario: Extensions exclude application frameworks

- **WHEN** inspecting `nova-extensions` public modules
- **THEN** it MUST NOT expose application config loaders, HTTP types, or IM channel connectors

### Requirement: Extensions trait implementations

`nova-extensions` SHALL provide default implementations for domain traits, including at minimum:

- `nova_core::ChatCompletion` (OpenAI-compatible adapter in `nova_extensions::completions`)
- `nova_extensions::preambles::TemplatedPreambler` with template substitutions and optional `PreambleSection` append (e.g. `SkillsSection`)
- `nova_extensions::skills` — load `SKILL.md` / `SKILL.toml`, render skills into system prompt XML
- `nova_core::Tool` (built-in tool types)
- `nova_core::ContextEngine` (`CompositeContextEngine` in `context/composite.rs`)

#### Scenario: TemplatedPreambler injects system prompt on assemble

- **WHEN** `CompositeContextEngine` is constructed with a `TemplatedPreambler` pipeline node
- **THEN** `assemble` MUST prepend or replace a leading `System` message from rendered template substitutions
- **AND** registered `PreambleSection` implementations (such as `SkillsSection`) MUST be appended after template substitution when non-empty

#### Scenario: SkillsSection renders authorization and available skills

- **WHEN** a non-empty skill list is passed to `SkillsSection`
- **THEN** the rendered fragment MUST include skills authorization guidance and an `<available_skills>` XML block

### Requirement: Extensions exclude application-constrained harness

Implementations that encode **one application's configuration contract** (for example reading `NOVA_OPENAI_*` environment variables for a CLI REPL) MUST NOT live in `nova-extensions`.

#### Scenario: EnvHarness is not in extensions

- **WHEN** inspecting `nova-extensions`
- **THEN** it MUST NOT define `EnvHarness` or `NOVA_OPENAI_*` env resolution as the primary configuration path
