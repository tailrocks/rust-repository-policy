# Install OK

Consumer-install fixture: a minimal adopter repo that materializes the
release (`modules/`, `profiles/`, `schemas/`) into `.cache/rust-policy/`,
extends the strict profile from its own `.alint.yml`, and calls
`rust-repository-policy check-gaps` via one `command_idempotent` rule.
`alint check` here MUST pass all 37 rules (36 native + `policy-gaps`).
