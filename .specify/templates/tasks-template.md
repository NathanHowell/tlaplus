---

description: "Task list template for feature implementation"
---

# Tasks: [FEATURE NAME]

**Input**: Design documents from `/specs/[###-feature-name]/`
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: The examples below include test tasks. Tests are OPTIONAL - only include them if explicitly requested in the feature specification.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

**Constitution Guardrails**:
- Include tasks for automated verification (Rust tests, TLC models, parity harness) before feature work starts.
- Add explicit tasks for selecting and integrating best-of-class Rust crates (`tracing`, `rayon`, `sled`, etc.) instead of direct Java-to-Rust translations.
- Add deterministic reproduction or benchmark tasks using `cargo test`, `cargo bench`, or dedicated scripts with recorded toolchain versions.
- Capture diagnostics/doc updates so Toolbox output and docs stay aligned with new Rust logging or CLI behavior.
- Ensure performance profiling/monitoring tasks exist when behavior can impact throughput or memory versus the Java baseline.
- Track references to upstream issue/plan IDs and note stakeholder communication or deprecation notices tied to migration steps.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Single project**: `src/`, `tests/` at repository root
- **Web app**: `backend/src/`, `frontend/src/`
- **Mobile**: `api/src/`, `ios/src/` or `android/src/`
- Paths shown below assume single project - adjust based on plan.md structure

<!-- 
  ============================================================================
  IMPORTANT: The tasks below are SAMPLE TASKS for illustration purposes only.
  
  The /speckit.tasks command MUST replace these with actual tasks based on:
  - User stories from spec.md (with their priorities P1, P2, P3...)
  - Feature requirements from plan.md
  - Entities from data-model.md
  - Endpoints from contracts/
  
  Tasks MUST be organized by user story so each story can be:
  - Implemented independently
  - Tested independently
  - Delivered as an MVP increment
  
  DO NOT keep these sample tasks in the generated tasks.md file.
  ============================================================================
-->

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and basic structure

- [ ] T001 Create project structure per implementation plan
- [ ] T002 Initialize [language] project with [framework] dependencies
- [ ] T003 [P] Configure linting and formatting tools
- [ ] T004 Install latest stable Rust toolchain and record version/`rust-toolchain.toml`
- [ ] T005 Create workspace-level `cargo fmt`, `cargo clippy` configurations and document commands

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

Examples of foundational tasks (adjust based on your project):

- [ ] T006 Setup database schema and migrations framework
- [ ] T007 [P] Implement authentication/authorization framework
- [ ] T008 [P] Setup API routing and middleware structure
- [ ] T009 Create base models/entities that all stories depend on
- [ ] T010 Configure error handling, `tracing`/logging infrastructure, and telemetry exporters
- [ ] T011 Establish parity harness comparing Rust outputs with legacy Java (temporary until Java removed)
- [ ] T012 Establish benchmark harness (`cargo bench`, TLC workload scripts) for performance tracking

**Checkpoint**: Foundation ready - user story implementation can now begin in parallel

---

## Phase 3: User Story 1 - [Title] (Priority: P1) 🎯 MVP

**Goal**: [Brief description of what this story delivers]

**Independent Test**: [How to verify this story works on its own]

### Tests for User Story 1 (OPTIONAL - only if tests requested) ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T013 [P] [US1] Rust unit/property tests in `crates/[name]/tests/`
- [ ] T014 [P] [US1] Integration test for [user journey] in `crates/[name]/tests/integration.rs`
- [ ] T015 [US1] TLC model/regression covering new state-space behavior in `tests/model/`
- [ ] T016 [US1] Golden parity script comparing Rust output vs Java baseline (remove when Java gone)

### Implementation for User Story 1

- [ ] T017 [P] [US1] Create Rust module `crates/[name]/src/[entity1].rs`
- [ ] T018 [P] [US1] Create Rust module `crates/[name]/src/[entity2].rs`
- [ ] T019 [US1] Implement service layer in `crates/[name]/src/lib.rs` (depends on T017, T018)
- [ ] T020 [US1] Expose CLI/FFI endpoint in `crates/[name]/src/bin/[tool].rs`
- [ ] T021 [US1] Add validation/error handling with idiomatic Rust error types (`thiserror`, `anyhow`)
- [ ] T022 [US1] Instrument `tracing` spans/metrics and document tag schema in docs/
- [ ] T023 [US1] Update spec/plan references with issue IDs and migration checklist entries
- [ ] T024 [US1] Run `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently

---

## Phase 4: User Story 2 - [Title] (Priority: P2)

**Goal**: [Brief description of what this story delivers]

**Independent Test**: [How to verify this story works on its own]

### Tests for User Story 2 (OPTIONAL - only if tests requested) ⚠️

- [ ] T025 [P] [US2] Rust contract test in `crates/[name]/tests/contract.rs`
- [ ] T026 [P] [US2] Integration test for [user journey] in `crates/[name]/tests/integration.rs`
- [ ] T027 [US2] TLC regression covering new state-space behavior in `tests/model/`
- [ ] T028 [US2] Extend golden parity harness for migrated functionality

### Implementation for User Story 2

- [ ] T029 [P] [US2] Create Rust module `crates/[name]/src/[entity].rs`
- [ ] T030 [US2] Implement service/engine logic in `crates/[name]/src/lib.rs`
- [ ] T031 [US2] Implement CLI/API wiring in `crates/[name]/src/bin/[tool].rs`
- [ ] T032 [US2] Integrate with User Story 1 components (if needed)
- [ ] T033 [US2] Update diagnostics documentation, Toolbox parser notes, and migration guide
- [ ] T034 [US2] Execute `cargo fmt`, `cargo clippy`, `cargo test`, parity harness

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently

---

## Phase 5: User Story 3 - [Title] (Priority: P3)

**Goal**: [Brief description of what this story delivers]

**Independent Test**: [How to verify this story works on its own]

### Tests for User Story 3 (OPTIONAL - only if tests requested) ⚠️

- [ ] T035 [P] [US3] Rust contract/property tests in `crates/[name]/tests/`
- [ ] T036 [P] [US3] Integration test for [user journey] in `crates/[name]/tests/integration.rs`
- [ ] T037 [US3] TLC regression covering new state-space behavior in `tests/model/`
- [ ] T038 [US3] Golden parity check or migration validation script

### Implementation for User Story 3

- [ ] T039 [P] [US3] Create Rust module `crates/[name]/src/[entity].rs`
- [ ] T040 [US3] Implement service logic in `crates/[name]/src/lib.rs`
- [ ] T041 [US3] Implement CLI/API feature in `crates/[name]/src/bin/[tool].rs`
- [ ] T042 [US3] Benchmark feature with agreed scenario using `cargo bench`/custom harness and capture results
- [ ] T043 [US3] Update migration status, release notes, and stakeholder comms
- [ ] T044 [US3] Run `cargo fmt`, `cargo clippy`, `cargo test`, parity harness, and perf benchmarks

**Checkpoint**: All user stories should now be independently functional

---

[Add more user story phases as needed, following the same pattern]

---

## Phase N: Polish & Cross-Cutting Concerns

**Purpose**: Improvements that affect multiple user stories

- [ ] TXXX [P] Documentation updates in docs/
- [ ] TXXX Code cleanup and refactoring
- [ ] TXXX Performance optimization across all stories
- [ ] TXXX [P] Additional unit tests (if requested) in tests/unit/
- [ ] TXXX Security hardening
- [ ] TXXX Run quickstart.md validation
- [ ] TXXX Remove or gate legacy Java/FFI shims slated for deprecation
- [ ] TXXX Update reproducibility checklist, parity harness status, and constitution guardrails

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can then proceed in parallel (if staffed)
  - Or sequentially in priority order (P1 → P2 → P3)
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P2)**: Can start after Foundational (Phase 2) - May integrate with US1 but should be independently testable
- **User Story 3 (P3)**: Can start after Foundational (Phase 2) - May integrate with US1/US2 but should be independently testable

### Within Each User Story

- Tests (if included) MUST be written and FAIL before implementation
- Models before services
- Services before endpoints
- Core implementation before integration
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel
- All Foundational tasks marked [P] can run in parallel (within Phase 2)
- Once Foundational phase completes, all user stories can start in parallel (if team capacity allows)
- All tests for a user story marked [P] can run in parallel
- Models within a story marked [P] can run in parallel
- Different user stories can be worked on in parallel by different team members

---

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together (if tests requested):
Task: "Contract test for [endpoint] in tests/contract/test_[name].py"
Task: "Integration test for [user journey] in tests/integration/test_[name].py"

# Launch all models for User Story 1 together:
Task: "Create [Entity1] model in src/models/[entity1].py"
Task: "Create [Entity2] model in src/models/[entity2].py"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently
5. Deploy/demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Deploy/Demo (MVP!)
3. Add User Story 2 → Test independently → Deploy/Demo
4. Add User Story 3 → Test independently → Deploy/Demo
5. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1
   - Developer B: User Story 2
   - Developer C: User Story 3
3. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
