# Linear workflow status: resolve, gate, transition

Shared by `spec-ticket`, `plan-ticket`, and `code-ticket` — anything that requires a ticket to be in a specific status before acting, and moves it to another status once its work is done.

**Resolve status names, don't hardcode IDs.** Call `mcp__claude_ai_Linear__list_issue_statuses` for the ticket's team and match the expected status by name, case-insensitively. Status names live in Linear's workflow config and can be renamed; a hardcoded ID would either silently no-op or fail confusingly when that happens.

**Gate on the precondition, don't force it.** After fetching the ticket, compare its current status to the one this skill expects. If it doesn't match, stop and tell the user what state the ticket is actually in — don't proceed, and don't move it there yourself. The ticket's status is a signal that a human (or a prior skill in this family) decided it was ready for this step; skipping the check risks acting on a ticket that isn't ready, or silently overriding someone else's workflow decision.

**Transition via `save_issue`'s `state` field, by name**, once the skill's work is done. This can be combined with the same `save_issue` call that writes content — no need for a separate round trip.
