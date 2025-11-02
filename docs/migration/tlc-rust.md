# TLC Rust Migration Notes

This document tracks behavior changes and feature retirements as we transition from the legacy Java TLC implementation to the Rust-native CLI.

## Retired Features

- **Mail notifications**: The legacy `util.MailSender` integration is removed. Automated environments should rely on external alerting (e.g., CI pipelines, log aggregation) instead of built-in email hooks.
- **`_PERIODIC` config keyword**: Previously allowed naming a zero-argument operator that TLC evaluated during each periodic scheduler cycle, aborting the run if it returned `FALSE`. The Rust CLI does not support this hook. To replicate the behavior, monitor NDJSON progress events (or final return codes) and terminate the process from an external supervisor when your assumption no longer holds.
- **`_RL_REWARD` config keyword and RL simulation mode**: Reinforcement-learning guided simulation (`tlc2.tool.Simulator.rl=true`) and its reward operator are not ported. Users who relied on this experimental mode should switch to the standard random simulator or integrate third-party fuzzing frameworks that consume the CLI.

## Tracking Guidance

- Flag any specs/configs that reference `_PERIODIC` or `_RL_REWARD` during migration reviews and work with spec owners to replace them.
- Update release notes to call out these removals so downstream tools (e.g., Toolbox plugins, CI wrappers) can adjust before the Rust TLC GA.
