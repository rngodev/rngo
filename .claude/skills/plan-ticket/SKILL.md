---
name: plan-ticket
description: Read a Linear ticket's spec (the requirements written by spec-ticket) and append a terse implementation plan that addresses each requirement — and any directly-relevant tech debt — then move the ticket from "Plan Pending" to "Plan Proposed". Use whenever the user gives a ticket identifier and asks to plan it, write an implementation plan, or figure out how to build what's specced — even if they just say "plan out ENG-123" or "LIN-88 has a spec, what's the plan."
---

# plan-ticket

Turns a ticket's requirements into a terse, codebase-grounded implementation plan.

## 1. Fetch and validate the ticket

Fetch the ticket with `mcp__claude_ai_Linear__get_issue` using its identifier. Confirm its current status is **Plan Pending** (match by name, case-insensitively, against `mcp__claude_ai_Linear__list_issue_statuses` for the ticket's team). If it's in a different status, stop and tell the user what state it's actually in rather than proceeding or force-moving it.

The ticket's current `description` is the spec written by `spec-ticket` — a list of terse, user-facing requirements. That's what the plan needs to satisfy.

## 2. Ground the plan in the actual codebase

Don't write a generic plan from the requirements alone — go look at the code. Find the files, modules, and existing patterns the change will touch (for this repo, start from the architecture notes in CLAUDE.md: `Spec` / `Dialect::parse_simulation` / `Simulation` / `Effect` / `Event` / `EventLog` in `crates/sim`, the CLI run loop and channel dispatch in `crates/cli`). A plan that could apply to any codebase isn't grounded enough yet.

While you're in there, note any tech debt that's directly in the way of this change (not tech debt in general) — if fixing it now is cheaper than working around it, say so in the plan; if it's a bigger, separable concern, name it but don't fold it in.

## 3. Write the plan

Append a `## Implementation Plan` section to the ticket's description (use `save_issue`'s `patch` with an `append` op so the requirements stay intact above it). Keep it terse:

- Address each requirement from the spec — either directly ("requirement X → change Y in file Z") or by grouping requirements that share an approach.
- Steps, not prose — short enough that someone skimming it in Linear gets the shape of the change in a few seconds.
- No code. This is a plan for a human (or a future `code-ticket` run) to execute, not the implementation itself.

**Example:**
Requirement: "A user can export all events from a completed run as a single file."
Plan bullet: "Add `rngo-cli run export` subcommand that reads `log.sqlite` for the given run and writes events as JSON lines to stdout or a file (`crates/cli/src/sim/run.rs`)."

## 4. Move the ticket

In the same or a following `save_issue` call, set `state` to **Plan Proposed**. Report the ticket URL back to the user for review.

## Note for future Linear skills

See the same note in `spec-ticket`'s SKILL.md — the state resolve/validate logic is duplicated between these two skills intentionally for now; factor it out if a third status-gated skill needs it too.
