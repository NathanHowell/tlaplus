# Backlog Remediation Workflow

This directory stores the authoritative remediation backlog for the TLC rewrite.
The backlog lives in `backlog.csv` so that status, ownership, and sign-off
metadata can be consumed by tooling or CI checks. Use this README as the
reference for how to triage, execute, and close remediation work.

## Status Codes

The following status codes are defined in the CSV and must be used verbatim.
Only advance items when the required documentation and evidence exist.

| Status | When to Use | Required Updates (in `backlog.csv`) | Exit Criteria |
| --- | --- | --- | --- |
| `OPEN` | Newly triaged backlog item awaiting a concrete remediation plan. | Provide `owner`, `priority`, `severity`, and populate `notes` with acceptance risks or open questions. | Remediation plan captured in `notes` and owner has acknowledged responsibility. |
| `IN_PROGRESS` | Active remediation work underway. | Update `notes` with progress summaries at least once per week. Include links to code branches, design docs, or blockers. | Change is ready for validation (tests, docs, and mitigation plan drafted) or enters `BLOCKED`. |
| `BLOCKED` | External dependency or risk prevents forward progress. | `notes` must capture the blocker, escalation path, and target unblock date. | Blocker resolved or leadership approves deprioritisation; transition back to `IN_PROGRESS` or `OPEN`. |
| `READY_FOR_REVIEW` | Implementation complete and awaiting maintainer validation. | Record regression/parity runs in `notes` along with links to PRs or artifacts. Ensure documentation updates are referenced. | Reviewer checklist below is satisfied; move to `RESOLVED` once reviewer approves. |
| `RESOLVED` | Fix merged and validated. Awaiting final maintainer sign-off. | `notes` should point to merged commits and verification evidence. Populate `last_updated` with the completion date. | Maintainer signs off in `maintainer_signoff` column (see T052) or item is re-opened. |
| `RETIRED` | Gap intentionally accepted with documented mitigation. | `notes` must link to the mitigation plan and stakeholder approvals. | Maintainer sign-off recorded and mitigation documentation published. |

## Workflow

1. **Intake & Triage**  
   - Add new entries as `OPEN` with clear summary, priority, severity, and
     nominated `owner`. Capture any known risks or dependencies in `notes`.
   - Verify the entry references supporting material (spec sections, parity
     ledger notes, etc.).

2. **Execution**  
   - When work begins, transition to `IN_PROGRESS` and keep `notes` updated with
     weekly progress, referencing commits, PRs, or docs.
   - If the item cannot progress, update to `BLOCKED`, record the blocking
     details, and tag escalation stakeholders.

3. **Validation**  
   - Move to `READY_FOR_REVIEW` once implementation, tests, and documentation
     updates are ready. Attach evidence to `notes`.
   - Reviewers apply the checklist below. If gaps remain, return to
     `IN_PROGRESS` with required follow-up captured in `notes`.

4. **Closure**  
   - After review approval, set status to `RESOLVED` and ensure regression,
     parity, and documentation artifacts are linked.
   - Maintenance leadership records sign-off in `maintainer_signoff` (handled in
     T052). For intentional gaps, document the mitigation and transition to
     `RETIRED`.

## Reviewer Checklist

Before approving an item for closure (`RESOLVED` or `RETIRED`), reviewers must
confirm all of the following:

- Evidence of regression and parity coverage is attached (logs, CI run, or
  parity ledger entry).
- Documentation updates or migration notes are linked when user-facing behavior
  changes.
- Telemetry, error codes, and user-facing output remain consistent with the
  constitution guardrails.
- `notes` include clear remediation summary and any residual risks or follow-up
  work.
- For `RETIRED` items, mitigation ownership and communication plan are recorded.
- Backlog CSV fields (`owner`, `priority`, `severity`, `last_updated`) reflect
  the latest state, enabling dashboards to surface accurate status.

If any checklist item fails, move the backlog entry back to `IN_PROGRESS` (or
`BLOCKED`) with corrective actions documented.
