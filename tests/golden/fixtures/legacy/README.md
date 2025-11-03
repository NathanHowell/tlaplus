# Legacy TLC Legacy Fixtures

This directory contains focused regression fixtures that reproduce
quirky legacy TLC behaviors. They allow the parity harness to stage
targeted runs that are difficult to cover with the canonical
regression-suite specs alone.

## `unicode/`

- **Purpose**: Exercises UTF-8 module sources, identifiers that mix
  Japanese, Greek, and mathematical symbols, and string literals that
  include emoji. It also documents the legacy CLI aliases that remain
  accepted for backwards compatibility even though the new `tlc`
  command surface now prefers dedicated subcommands.
- **Artifacts**:
  - `UnicodeLegacy.tla` / `UnicodeLegacy.cfg` — minimal PlusCal/TLA+
    model plus configuration harnessing the Unicode inputs.
  - `fixture.toml` — metadata consumed by upcoming parity tasks. It
    enumerates the deprecated CLI flags that should accompany this
    fixture (e.g., `-noTE`, `-tool`, `-nowarning`) so the harness can
    validate that the Rust CLI preserves their legacy semantics.

Additional legacy scenarios live under sibling directories as they
come online (see tasks T068+).
