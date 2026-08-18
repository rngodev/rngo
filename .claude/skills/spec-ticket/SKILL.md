---
name: spec-ticket
description: Expand a Linear ticket's initial idea into terse, purely user-facing requirements (no implementation details), then move the ticket from "Spec Pending" to "Spec Proposed". Use whenever the user gives a ticket identifier and asks to spec it, write requirements for it, or turn the idea into a spec — even if they just say "spec out ENG-123" or "the idea's written up on LIN-88, can you turn it into requirements."
---

# spec-ticket

Turns a rough idea sitting on a Linear ticket into a terse requirements doc, written strictly from the user's point of view.

## 1. Fetch and validate the ticket

Fetch the ticket with `mcp__claude_ai_Linear__get_issue` using its identifier. Per `.claude/skills/_shared/linear-status.md`, confirm its current status is **Spec Pending** before proceeding.

The ticket's current `description` is the initial idea — that's your only input. If it references other tickets or context that seem necessary to understand the ask, pull those too rather than guessing.

## 2. Write the requirements

Rewrite the description as a list of terse requirements. The test for each one: could a developer read it and understand what the user needs, without it telling them how to build it?

- Purely from the user's perspective — what they can do, see, or rely on. No file names, functions, schemas, libraries, endpoints, or architecture.
- Terse. One line per requirement where possible; cut anything that doesn't change what gets built or verified.
- Concrete enough to verify — "the export includes every effect event" not "the export should be complete."
- If the original idea was already vague, don't invent scope to fill the gap — surface the open question as a requirement that names the ambiguity, rather than silently deciding for the user.

**Example:**
Idea: "users should be able to export their run's events"
Requirements:
- A user can export all events from a completed run as a single file.
- The export includes every effect event and every error event, in the order they occurred.
- The exported file is usable on its own, without also needing the run's spec.json.

Replace the ticket's `description` with the requirements via `mcp__claude_ai_Linear__save_issue` (Linear keeps prior versions in its edit history, so the original idea isn't lost by overwriting it).

## 3. Move the ticket

In the same or a following `save_issue` call, set `state` to **Spec Proposed** (see `.claude/skills/_shared/linear-status.md`). Report the ticket URL back to the user so they can review it in Linear — this skill proposes the spec, it doesn't get final sign-off itself.
