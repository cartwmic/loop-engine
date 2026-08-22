# Report artifact (generate-PRD)

Write `report.json` as a cited conclusion grounded in verified claims. The provider does not judge conclusion quality.

The conclusion must emit a schema-valid living markdown PRD candidate for the current repository, plus per-requirement repository evidence citations. Each proposed requirement must carry reviewable repository evidence — a locator and extract an independent reviewer can open in this checkout.

Proposed live records use the published grammar. Canonical live-record spelling:

```markdown
### LE-<n>: <title>
- Status: live
- Coverage: e2e/journey
```

Optional additional contract class, only when a live contract surface exists:

```markdown
### LE-<n>: <title>
- Status: live
- Coverage: e2e/journey, contract
```

`<n>` is `[1-9][0-9]*` (no leading zeros). Candidate IDs are proposals; no ID is authoritative until a human accepts and commits it to the living PRD. Do not use Compass `PREFIX-N` headings or `@spec:` citation spelling.

After synthesize, also write the human-facing candidate markdown to `prd-candidate.md` under the run artifact root. A human must accept or reject that candidate before any commit to `docs/PRD.md`. Do not auto-commit.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when the conclusion is materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `verification_revision`: must equal current `verification.json` revision when the `completed` gate is checked.
- `conclusion`: the cited answer to the brief question — the candidate markdown plus per-requirement repository evidence citations.
- `citations`: one or more `{claim_id, source_id}` pairs tying the conclusion to verified claims and gathered sources.

Report-local defects stay in synthesize: edit and recheck `report.json`, then retry checked `completed`. Nearest check-free `revise` is verification-owned only; use `revise-sources` for sources-owned defects or `revise-brief` for brief-owned defects. Do not waive known defects.
