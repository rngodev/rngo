---
name: code-ticket
description: Turn a Linear ticket into an implemented, reviewable pull request end to end — fetch the ticket by its identifier (e.g. ENG-123), implement the change it describes, then create an appropriately named branch, commit, push, and open a GitHub PR that links back to the ticket. Use this whenever the user gives a Linear ticket/issue number or identifier and asks to work it, ship it, start it, or open a PR for it — even if they just say something like "grab ENG-431 and put up a PR" or "let's knock out LIN-88."
---

# Linear ticket → PR

Given a Linear ticket, this skill takes it from "assigned" to "PR open for review": fetch the ticket, implement the change, then package it up as a branch/commit/PR that Linear can auto-link back to the ticket.

## 1. Resolve the ticket

Ask for the ticket identifier if not given (e.g. `ENG-123`). Fetch it with `mcp__claude_ai_Linear__get_issue` using the identifier directly as `id`. This returns the title, description, `gitBranchName` (Linear's own suggested branch name for the issue — matching the format Linear's GitHub integration recognizes for auto-linking), and `url`.

If the user gives a bare number with no team prefix, ask which team it belongs to before looking it up — Linear identifiers always include the team key (e.g. `ENG-123`), and `get_issue` needs that to resolve correctly.

Confirm its current status is **Todo** (match by name, case-insensitively, against `mcp__claude_ai_Linear__list_issue_statuses` for the ticket's team — status names can drift from what's assumed here). If it's in a different status, stop and tell the user what state it's actually in rather than proceeding or force-moving it.

Read the ticket's description carefully; it's your spec. If it references other tickets, designs, or comments that seem load-bearing, pull those too (`list_comments`, `get_issue` on referenced tickets) rather than guessing at intent.

## 2. Start from a clean base

Before creating the branch, make sure the working tree is clean and you're building on an up-to-date default branch (`git status`, `git fetch`, then branch from `origin/main` or equivalent) — the repo may be checked out mid-way through unrelated work, and you don't want to bundle that into the ticket's PR. If the working tree isn't clean, stop and ask the user how they want to handle it rather than stashing or discarding anything automatically.

## 3. Create the branch

Use the `gitBranchName` Linear returned. This already embeds the ticket identifier in the format Linear's GitHub integration looks for, so the PR auto-links without any extra step. Only fall back to hand-slugifying `<identifier>-<slugified-title>` if Linear didn't return one.

## 4. Implement the change

Make the code changes the ticket describes, following the conventions already established in the codebase (check for a CLAUDE.md or similar and follow it — e.g. formatting/linting commands, test commands). Run the project's format/lint/test commands before committing, the same way you would for any other change in this repo — a ticket-driven change isn't exempt from the codebase's normal quality bar.

If the ticket is ambiguous or underspecified in a way that would force you to guess at product intent, pause and ask rather than picking an interpretation silently — it's a lot cheaper to ask now than to redo it after review.

## 5. Commit, push, and open the PR

- Commit with a message in this repo's existing style (check `git log` for tone/format — don't impose a convention that isn't already there, e.g. don't add Conventional Commit prefixes if the repo doesn't use them).
- Push the branch to `origin`.
- Open the PR with `gh pr create`. Title it after the change (not just the ticket title verbatim, unless they already match). In the body, include a magic-word reference so Linear auto-closes the ticket on merge (e.g. `Fixes ENG-123`), plus a short summary of what changed and why, written for a human reviewer, not a ticket-restatement.
- Move the ticket to **In Review** via `mcp__claude_ai_Linear__save_issue` now that there's a PR open against it.

Report back the PR URL. Don't merge it — opening it for review is the end of this skill's job.

## Notes for future Linear skills

Other skills in this family (pulling/updating tickets) should keep using the ticket's identifier (e.g. `ENG-123`) as the canonical reference passed to `get_issue`/`save_issue`, and the same magic-word convention (`Fixes <identifier>`, `Closes <identifier>`) for anything that links Linear issues to GitHub PRs/commits. There's only one skill using these conventions today, so they're inlined here rather than factored into a shared reference — if a third Linear skill needs the same lookup/linking logic, that's the point to pull it into a shared `references/linear-conventions.md` alongside these skills.
