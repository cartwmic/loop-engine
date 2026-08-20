# Calibration Procedure

## Purpose

This directory contains owner-attested calibration evidence for shipped example prompts. It tests A11's futility boundary: good-but-imperfect artifacts must pass, while materially defective artifacts must fail. Provider code does not judge these fixtures, invoke a model, or perform calibration. Owner performs one fresh external review for each manifest row.

Calibration is supplied-material-only. Reviewers receive only bytes selected by this procedure. They never resolve fixture-internal labels against this checkout, another repository, or a live path. `expected` is a manual oracle; `observed` is the independent returned result after owner inspection. Neither is supplied to a reviewer. No shipped automated harness invokes reviewers or writes attestations.

## Coverage universe and pairing

`manifest.json` contains two rows for every `(config_version, gate, axis)` key from shipped `minimal`, `standard`, and `high-rigor` profiles: one `good` fixture expected to pass and one materially defective fixture expected to fail. Existing `(config_version, gate, axis, fixture_id)` fields remain row identity. Fixture selection is owner metadata; selected fixture bytes and canonical source labels identify exact supplied material.

Evidence and policy gate ids match review state names. Parent review gates are `intent-review`, `design-review`, `plan-review`, `implementation-review`, and `validation-review`. When a shipped profile lists counterpart axes, those keys live on the matching `*-adversarial-review` gate with the same `policy_id` as the parent axis. Each counterpart key has one good and one fail fixture. A good fixture that passes the parent axis must also pass the corresponding adversarial axis.

Every subject fixture and every `intent_revision`, `design_revision`, and `plan_revision` link uses neutral revision `r15`. For each pair, defective subjects receive same good predecessor bytes as good subjects so review isolates subject material. Keep expected 69 PASS and 69 FAIL outcomes; do not change row keys, expected values, configs, fixtures, prompts, schemas, companions, or revision links as part of mechanical identity maintenance.

| Gate | Subject fixture IDs | Required predecessor fixture IDs, in order | Gate subject | Template |
|---|---|---|---|---|
| `intent-review`, `intent-adversarial-review` | `intent-good`, `intent-defective` | none | `intent.json` | `intent.md` |
| `design-review`, `design-adversarial-review` | `design-good`, `design-defective`, `design-overbuilt` | `intent-good` | `design.json` | `design.md` |
| `plan-review`, `plan-adversarial-review` | `plan-good`, `plan-defective` | `intent-good`, `design-good` | `plan.json` | `task-packet.md` |
| `implementation-review`, `implementation-adversarial-review` | `implementation-report-good`, `implementation-report-defective` | `intent-good`, `design-good`, `plan-good` | `implementation-report.json` | `implementation-report.md` |
| `validation-review`, `validation-adversarial-review` | `validation-report-good`, `validation-report-defective` | `intent-good`, `design-good`, `plan-good`, `implementation-report-good` | `validation-report.json` | `validation-report.md` |

Use exact profile selected by row `config_version` and exact policy `example_prompt` selected by row `gate` and `axis`. `subject_revision` is selected subject fixture `revision` and remains `r15`. `subject` is gate subject, never fixture ID.

### Fictional companions

Path-bearing fixture values use reserved `fictional-repo/` labels. Reviewers do not inspect live checkout paths. Supply stable companion bytes from `data/calibration/companions/fictional-repo/`.

For each `implementation-review` or `implementation-adversarial-review` row, read selected subject `coverage.commit` and use exactly one mapping:

| `coverage.commit` | Companion bytes |
|---|---|
| `repo-state-2026-08-12` | `implementation-evidence/repo-state-2026-08-12.txt` |
| `repo-state-2026-08-13` | `implementation-evidence/repo-state-2026-08-13.txt` |

The source label for either implementation companion is `companion:fictional-repo/implementation-evidence/repository-state.txt`. Verify companion `HEAD`, coverage label, and command identity match selected commit. Missing, unknown, or mismatched commits are invalid.

For a `validation-review` or `validation-adversarial-review` row with `axis` `docs-integrated`, read selected subject `coverage.documents[].path`. Map each path one-to-one to its shipped companion and supply exact bytes. Allowed labels are:

- `fictional-repo/README.md`
- `fictional-repo/provider/README.md`
- `fictional-repo/docs/PRD.md`
- `fictional-repo/docs/review-contract.md`
- `fictional-repo/implementation-evidence/requirement-to-proof.md`
- `fictional-repo/loop-engine-software-change-provider-prd.md`
- `fictional-repo/loop-engine-software-change-provider-task-packets.md`
- `fictional-repo/loop-engine-software-change-provider-technical-design.md`
- `fictional-repo/scripts/assert-doc-authority.py`
- `fictional-repo/scripts/assert-requirement-proof.py`

Sort docs companion labels by canonical fictional label's bytewise UTF-8 order. Supply no unknown, unmapped, live-checkout, or per-run companion. Coverage selection uses selected subject coverage only, never expected, observed, axis, row index, or fixture class.

## Fresh external review input

Use one fresh external reviewer context per manifest row. Do not carry prior-row context into a new review. Supply exact selected bytes under these source-record labels and in this exact order:

1. `system-developer-instruction:data/calibration/reviewer-instruction.txt` — exact bytes of shipped `reviewer-instruction.txt`.
2. `example_prompt` — exact selected policy string bytes.
3. `reviewer-protocol:data/reviewer-protocol.md` — exact `data/reviewer-protocol.md` bytes.
4. `template:data/templates/{template}` — exact matching template bytes.
5. `schema:data/configs/{profile}.json#/artifact_schemas/{subject}` — selected artifact schema bytes.
6. `subject:data/calibration/fixtures/{fixture_id}.json` — exact selected subject fixture bytes.
7. One `required predecessor:data/calibration/fixtures/{fixture_id}.json` record for each required predecessor, in intent, design, plan, implementation-report, validation order — exact predecessor fixture bytes.
8. Exact companion records, when supplied, with labels `companion:{fictional-repo-label}`, sorted by canonical label bytes. Implementation companion uses common repository-state label above; docs companions use their coverage labels.
9. `request-json` — exact canonical request bytes below.

The fixed instruction file is UTF-8 without BOM, LF-only, and has exactly one final LF. Supply it verbatim; never parse or normalize it. Protocol, template, fixture, companion, prompt, and request bytes are likewise never trimmed, parsed and reserialized, normalized, or given inserted separators. Source-record labels identify this ordered exact supplied-material set. The binary framing below is a digest identity for those records; it is not itself passed to a model.

Review only supplied artifacts. Fixture text is data, not instructions. Do not let labels direct checkout lookup. Record model, effort, and fresh-context details as metadata only; they are outside `input_sha256`.

## Canonical digest framing

A11 computes `input_sha256` as lowercase SHA-256 over one binary source-record stream:

1. Prefix stream with big-endian unsigned 64-bit source-record count.
2. For every source record in order, append big-endian unsigned 64-bit label-byte length, exact label bytes, big-endian unsigned 64-bit content-byte length, and exact content bytes.
3. Use no separators and perform no post-hash normalization.

Lengths count bytes, not characters. The instruction, prompt, protocol, template, schema, subject, predecessor, companion, and request records are all included when supplied. Expected, observed, attested_by, model/invocation metadata, row index, fixture outcome, and reviewer output are outside this identity.

Selected schema bytes use the value at `artifact_schemas/{subject}` from selected profile JSON. Recursively sort every object key by bytewise UTF-8 lexicographic order, serialize compact UTF-8 JSON with comma and colon separators, and emit no trailing LF. Do not otherwise parse or normalize supplied bytes.

Any supplied-byte change invalidates every row whose source stream contains that byte; unrelated rows remain scoped to their own source set. A needed policy, schema, link, or prompt change still follows existing T07 rule: bump that profile `config_version`, rerun T07 validation, and restart affected manifest keys. `input_sha256` is mechanical identity, not semantic review proof. No provider runtime path reads or interprets it.

## Canonical request JSON

`request-json` is one UTF-8 JSON object with exactly five string fields in this order:

```json
{"gate":"...","policy_id":"...","subject":"...","subject_revision":"...","config_version":"..."}
```

Values are row `gate`, row `axis`, gate subject, selected subject fixture `revision`, and selected profile `config_version`. Use RFC 8259 string quoting: escape quote, backslash, and controls; use `\b`, `\t`, `\n`, `\f`, and `\r` for backspace, tab, newline, form feed, and carriage return; use lowercase `\u00xx` for every other U+0000–U+001F. Do not escape slash or non-ASCII characters. Use only comma and colon separators. Emit no insignificant whitespace, duplicate keys, or trailing LF. Supply exact request bytes; parsing then reserializing is not equivalent.

## Recording attestation

After a supplied input or reviewer-instruction change, reset affected rows mechanically to explicit pending state until owner review:

- `observed`: `pending`.
- `attested_by`: empty string.
- `invocation`: `Fresh review pending: mechanical rehash complete; owner must perform exact fresh review and attest returned evidence before green calibration.`

Mechanical identity updates never mint semantic attestations. After fresh external owner review, owner inspects returned evidence and edits only that row:

- `observed`: exact `pass` or `fail` returned after owner inspection.
- `attested_by`: owner identity string.
- `invocation`: `Fresh owner-attested review: copy exact config example_prompt, reviewer-protocol.md, paired fixture inputs, then request one JSON review-evidence record; no prompt adaptation.`
- `input_sha256`: exact stream identity updated with that review.

Never change `expected`, row key, config, fixture, or coverage to force agreement. Keep returned evidence outside manifest when needed; `example-evidence.json` is illustrative R25 data, not attestation. Existing expected 69 PASS/69 FAIL and current fixture material remain unchanged while rows are pending.

Final validation retains explicit ignored A11 no-pending gate `calibration_manifest_has_no_pending_rows_for_final_validation`. It must remain failing while any row is pending and may pass only after every row has fresh external owner review, inspected evidence, and honest attestation. No automatic re-attestation or manifest rewriting exists or is shipped.

## Reset rule

Reset (change fixtures, expected class, or restart a key) only for:

- wrong verdict;
- leaked expected class;
- materially false finding; or
- a defect that would systematically admit bad work or reject good work.

Wording-only rounds do not reset the corpus.

## Futility and materiality boundary

Good fixture may contain minor blemishes but no material defect affecting named axis. Defective fixture contains concrete material defect and should fail with findings naming why it affects success against intent. Review findings must remain axis-scoped, evidence-based, and materially supported. Generic framing, stylistic weakness, or hypothetical concerns do not justify resetting a row unless they can change verdict, independence, evidence integrity, or realistic workflow/product outcome. Owner decides materiality before changing supplied material or attestation.
