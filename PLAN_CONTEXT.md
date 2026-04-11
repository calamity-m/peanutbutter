# Plan Context

This file defines how tracked plan bundles work in this repository.

The goal is simple: keep planning and implementation close together so the current state of the work is visible in files, not trapped in chat history.

## Purpose

Use `plans/<slug>/README.md` for the task-level summary, sequencing, and cross-part status.

Use `plans/<slug>/part-*.md` files for the active working notes, checklists, discoveries, validation, and handoff for each part.

## File Layout

- `PLAN_CONTEXT.md`
- `plans/<slug>/README.md`
- `plans/<slug>/part-01*.md`
- `plans/<slug>/part-02*.md`

## Update Rules

- Update the active part file after material progress.
- Update the active part file when new discoveries change the shape of the work.
- Update the task plan `README.md` at `plans/<slug>/README.md` when part status, blockers, sequencing, or acceptance changes.
- Update the task plan `README.md` at `plans/<slug>/README.md` after each review pass with findings and resulting decisions.
- Record important decisions in the plan files instead of leaving them only in chat.
- If implementation diverges from the written plan, bring the docs back into sync immediately.
- Do not update the repo root `README.md` as part of plan maintenance unless the user explicitly changes the product specification.

## Status Values

- `planned`: work not started yet
- `in-progress`: currently being worked
- `blocked`: cannot proceed until a dependency or decision is resolved
- `done`: implemented and validated to the degree stated in the part file

## Working Agreement

- Prefer 2-5 parts for most tasks.
- Keep each part independently meaningful and verifiable.
- Run the four review passes on substantive plans unless the task is too small to justify them.
- Put validation notes in every part file.
- End each part with a concrete next handoff.
- Keep `plans/<slug>/README.md` as the single place where a reader can see the whole task status quickly.

## Repo-Specific Notes

- Keep tracked implementation plans under `plans/`.
- Treat `README.md` as the product specification unless the user explicitly overrides it.
- Default to a single Rust crate with internal modules while the repo is still small; do not introduce a workspace or service split without a concrete reason.
- Record both automated Rust validation and manual shell or TUI smoke checks when terminal behavior cannot be covered by tests alone.
- When `examples/` diverge from `README.md`, record the mismatch in the active plan and resolve toward the README unless a follow-up decision says otherwise.
