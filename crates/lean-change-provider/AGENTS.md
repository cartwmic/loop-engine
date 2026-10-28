# lean-change-provider agent instructions

## Scope and authority

Root `AGENTS.md` governs checkout operations and authority; this guide adds crate-local procedure only.
Product summary is in [README.md](README.md). Requirements are LE-202 to LE-209 in the root PRD.

## Workflow

Load the [skill](skills/using-lean-change-provider/SKILL.md) before driving a run. The supported
commands are `setup`, `verify`, `intent-packet`, `review-packet` and stdin describe/evaluate; there is no `data-dump`
and no shipped data.

Profiles b and c share one code base. Do not fork the code for c: c is the review flag off. A new gate needs a
unit test that fails without it and a row in `scripts/lean-change-mutations.py`. The core (`src/lib.rs`: model, workflow, gates) does no process, file or git access; the binary supplies the repository tree through the `TreeIdentity` port and a unit test enforces the boundary. Put new I/O in `run.rs` or `packet.rs`, never in the core. Keep the question lists in
`src/model.rs` (the five intent-check questions and the six review questions) identical to the PRD text (LE-207 and LE-209).

Check from the checkout root: `cargo test -p lean-change-provider`, then the journey and the mutation script named in the root `AGENTS.md`.
