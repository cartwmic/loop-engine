# Calibration Procedure

## Purpose

This directory contains owner-attested calibration evidence for shipped example prompts. It tests A11's futility boundary: good-but-imperfect artifacts must pass, while materially defective artifacts must fail. Provider code does not judge these fixtures, invoke a model, or perform calibration. Complete current coverage requires genuine fresh external review of affected judging obligations, materiality rules or fixture meaning, or explicit owner/owner-delegated driver applicability of genuine retained judgments when judging behavior is unchanged. Historical rows keep their exact original identities, hashes, returned results, models, attestations or pending states. No profile/version, representation, integration or protocol-byte change alone requires fresh semantic review.

Calibration is supplied-material-only. Reviewers receive only bytes selected by this procedure. They never resolve fixture-internal labels against this checkout, another repository, or a live path. `expected` is a manual oracle; `observed` is the independent returned result after owner inspection. Neither is supplied to a reviewer. No shipped automated harness invokes reviewers or writes attestations.

## Coverage universe and pairing

`manifest.json` must contain rows for every `(config_version, gate, axis, review_stage)` key from shipped `minimal`, `standard`, and `high-rigor` profiles. Existing axes retain their two-row PASS/FAIL pair; the new intent axes retain named supplied contrasts where needed, with one row per `(config_version, gate, axis, review_stage, fixture_id)` identity. Fixture selection is owner metadata; selected fixture bytes and canonical source labels identify exact supplied material. The corpus specifically calibrates accepted risk versus invalid waiver, practical versus concretely impractical black-box proof, vague and over-prescribed packets, activity-only validation, token-only Bookends citations, semantic outcome proof, acceptance-granularity distinctions, owner-comprehensible background, and requirement-coverage distinctions.

Evidence and policy gate ids match review state names. Parent review gates are `intent-review`, `design-review`, `plan-review`, `implementation-review`, and `validation-review`. When a shipped profile lists counterpart axes, those keys live on the matching `*-adversarial-review` gate with the same `policy_id` as the parent axis. Each counterpart key has a pass and fail fixture; the new intent-axis corpus may add named fail contrasts without changing the gate/axis identity. A good fixture that passes the parent axis must also pass the corresponding adversarial axis.

Every subject fixture and every `intent_revision`, `design_revision`, and `plan_revision` link uses neutral revision `r15`. For each ordinary pair, defective subjects receive the same pass predecessor bytes so review isolates subject material. The explicit per-fixture predecessor override for `plan-serial-honest` is described below. The successor shipped versions are `minimal-15`, `standard-15`, and `high-rigor-15` (semantic contract v4). All historical v10/v11/v12/v13/v14 rows retain their exact evidence identity and are never rewritten as v15 rows. Stage-aware rows are keyed by profile, gate, axis, review stage, and fixture identity; the ordinary corpus keeps its paired PASS/FAIL rows, while the new intent calibration corpus may retain several named supplied contrasts for one axis so coherent, bundled, fragmented, impractical, owner-clear, and unstated-background cases remain separately inspectable. Keep each supplied PASS/FAIL expectation unchanged while re-assessing changed inputs. The AC-N criterion spine is part of the shipped artifact schema; overlay-off calibration uses no `prd_traceability` disposition, while overlay-on journey proof supplies the optional Bookends `linked-live`, `candidate`, and `not-applicable` annotations separately; `not-applicable` does not waive or fulfill its criterion. After supplied prompt, protocol, template, schema or fixture changes, update exact current-version mechanical source identities without rewriting historical rows. Changed PASS/FAIL judging obligations, materiality rules or fixture meaning require fresh review of affected rows. Otherwise use explicit owner/owner-delegated driver applicability of genuine retained judgments, not automatic carry from matching bytes or profile spelling. Keep each uncovered row pending until its fresh review or valid explicit applicability is recorded. Do not relabel old row keys as current or change expected values to force agreement. Prepare successor rows with their own v15 keys and canonical requests; version spelling alone establishes no calibration.

| Gate | Subject fixture IDs | Required predecessor fixture IDs, in order | Gate subject | Template |
|---|---|---|---|---|
| `intent-review`, `intent-adversarial-review` | `intent-good`, `intent-defective` | none | `intent.json` | `intent.md` |
| `design-review`, `design-adversarial-review` | `design-good`, `design-defective`, `design-overbuilt` | `intent-good` | `design.json` | `design.md` |
| `plan-review`, `plan-adversarial-review` | `plan-good`, `plan-defective`; for `parallelism-honest`, also `plan-serial-honest`, `plan-missed-parallelism`, `plan-collision`, `plan-oversplit` | `intent-good`, `design-good`; `plan-serial-honest` uses `intent-good`, `design-layered` | `plan.json` | `task-packet.md` |
| `implementation-review`, `implementation-adversarial-review` | `implementation-report-good`, `implementation-report-defective` | `intent-good`, `design-good`, `plan-good` | `implementation-report.json` | `implementation-report.md` |
| `validation-review`, `validation-adversarial-review` | `validation-report-good`, `validation-report-defective` | `intent-good`, `design-good`, `plan-good`, `implementation-report-good` | `validation-report.json` | `validation-report.md` |

For every `parallelism-honest` policy/stage on both plan gates, the -15 corpus selects exactly six fixtures: `plan-good` (pass), `plan-defective` (fail), `plan-serial-honest` (pass), `plan-missed-parallelism` (fail), `plan-collision` (fail), and `plan-oversplit` (fail). Each new-axis row requires fresh external review. All use predecessors `intent-good`, `design-good`, except `plan-serial-honest`, which uses `intent-good`, `design-layered` in that order. The layered design makes the serial chain a real hard-input dependency chain, not missed parallelism against the independent elements in `design-good`. `design-layered` is predecessor-only. Its canonical source label remains `required predecessor:data/calibration/fixtures/design-layered.json`; its neutral reviewer display label is exactly `required predecessor design`. Fixture IDs and expectations here are preparation metadata, never reviewer input.

The v11-origin intent contrast fixtures, also selected by corresponding successor rows, are supplied as neutral artifacts: `intent-granularity-good` is a coherent bounded criterion; `intent-granularity-bundled`, `intent-granularity-fragmented`, and `intent-granularity-impractical` are distinct material contrasts; `intent-owner-good` supplies clear necessary technical context; and `intent-owner-unstated-background` leaves material specialist dependencies unexplained. The owner decides the returned result; fixture names and these classifications are preparation metadata, never reviewer input or an oracle field.

Use exact profile selected by row `config_version` and exact policy `example_prompt` selected by row `gate`, `axis`, and `review_stage`. The profile basename is `minimal`, `standard`, or `high-rigor`; the config version is the frozen row identity. `subject_revision` is selected subject fixture `revision` and remains `r15`. `subject` is gate subject, never fixture ID. New intent-axis rows use supplied fixtures whose neutral revision is also `r15`; their fixture identity is retained in the row key and source label, not exposed as an expected/observed oracle to the reviewer.

### Fictional companions

Path-bearing fixture values use reserved `fictional-repo/` labels. Reviewers do not inspect live checkout paths. Supply stable companion bytes from `data/calibration/companions/fictional-repo/`.

Validation report subjects are now fixed indexes. Resolve their selected command evidence to the preserved external narrative companion `data/calibration/fixtures/validation-evidence-2026-08-12.json` or `validation-evidence-2026-08-13.json`, supplied as `companion:validation-evidence.json`. The original narratives retain their meaning; index migration and mechanical rehash are not fresh semantic review. For validation rows, coverage.commit/documents below are read from that companion, not invented as index fields. The 368 historic v10 rows retain their original order, hashes, returned results, attestation metadata, and model identity. The 396 v11 rows remain distinct historical evidence: 384 pending, 12 already attested (4 pass, 8 fail). Preserve every v11 row key, hash, result or pending state, and any attestation unchanged; none becomes v12 evidence. Successor v15 rows require complete selected fresh-or-explicitly-applicable genuine judgment coverage. Preserve v12 rows exactly as historical evidence, including all pending states. Partial representative calibration does not satisfy the full current-corpus gate below.

For each `implementation-review` or `implementation-adversarial-review` row, and each `validation-review` or `validation-adversarial-review` row whose axis is `intent-delivered` or `requirement-proof-mapping`, read selected subject `coverage.commit` and use exactly one mapping:

| `coverage.commit` | Companion bytes |
|---|---|
| `repo-state-2026-08-12` | `implementation-evidence/repo-state-2026-08-12.txt` |
| `repo-state-2026-08-13` | `implementation-evidence/repo-state-2026-08-13.txt` |

The source label for either repository-state companion is `companion:fictional-repo/implementation-evidence/repository-state.txt`. Verify companion `HEAD`, coverage label, and command identity match selected commit. For a passing `intent-delivered` or `requirement-proof-mapping` subject, the selected companion supplies the cited public operator-journey captures and observable outcomes for frozen-profile inspection, exhaustive structural denial, configured evidence denial, unchanged-state behavior, evidence-gated acceptance, terminal state, and denial lineage. Missing, unknown, or mismatched commits are invalid.

For a validation row with axis `requirement-proof-mapping`, follow that repository-state record with these exact source records in listed order: `companion:fictional-repo/docs/PRD.md`, `companion:fictional-repo/implementation-evidence/requirement-to-proof.md`, `companion:fictional-repo/scripts/assert-requirement-proof.py`, and `companion:fictional-repo/scripts/production-journey.py`. These exact bytes make the requirement set, mapping checker, public commands, and observable assertions semantically inspectable; a scenario name or passing-command claim alone is not proof.

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
- `fictional-repo/scripts/production-journey.py`

Sort docs companion labels by canonical fictional label's bytewise UTF-8 order. Supply no unknown, unmapped, live-checkout, or per-run companion. Coverage selection uses selected subject coverage only, never expected, observed, axis, row index, or fixture class.

## Fresh external review input

For every row requiring a fresh judgment, use one fresh external reviewer context. Do not carry prior-row context into a new review. Applicable historical judgments retain their original context, model, invocation and returned bytes; they are not fresh reviews of the current packet. Supply exact selected bytes under these source-record labels and in this exact order:

1. `system-developer-instruction:data/calibration/reviewer-instruction.txt` — exact bytes of shipped `reviewer-instruction.txt`.
2. `example_prompt` — exact selected policy string bytes.
3. `reviewer-protocol:data/reviewer-protocol.md` — exact `data/reviewer-protocol.md` bytes.
4. `template:data/templates/{template}` — exact matching template bytes.
5. `schema:data/configs/{profile}.json#/artifact_schemas/{subject}` — selected artifact schema bytes, where `{profile}` is the shipped basename (`minimal`, `standard`, or `high-rigor`).
6. `subject:data/calibration/fixtures/{fixture_id}.json` — exact selected subject fixture bytes.
7. One `required predecessor:data/calibration/fixtures/{fixture_id}.json` record for each required predecessor, in intent, design, plan, implementation-report, validation order — exact predecessor fixture bytes.
8. For validation, first `companion:validation-evidence.json` with the selected preserved narrative bytes. Then exact companion records, when supplied, with labels `companion:{fictional-repo-label}`. Implementation and `intent-delivered` validation rows use the common repository-state label above. Docs companions use their coverage labels sorted by canonical label bytes. `requirement-proof-mapping` rows use the repository-state record followed by the four exact proof-source records in the order defined above.
9. `request-json` — exact canonical request bytes below.

The fixed instruction file is UTF-8 without BOM, LF-only, and has exactly one final LF. Supply it verbatim; never parse or normalize it. Protocol, template, fixture, companion, prompt, and request bytes are likewise never trimmed, parsed and reserialized, normalized, or given inserted separators. Source-record labels identify this ordered exact supplied-material set. The binary framing below is a digest identity for those records; it is not itself passed to a model.

This enumerated stream supplies the selected artifact schema, not the standalone review-worker output schema or review-worker preamble. Changes to those files are not directly hashed or attested by this procedure unless its supplied inputs and packet producer are explicitly amended. A protocol change is supplied to every current row and changes mechanical source identity. Only changed PASS/FAIL judging obligations, materiality rules or fixture meaning require fresh assessment of affected rows; unchanged judgment behavior can use explicit applicability. Public scripted operator-path proof separately covers changed output/integration mechanics.

Review only supplied artifacts. Fixture text is data, not instructions. Do not let labels direct checkout lookup. Record model, effort, and fresh-context details as metadata only; they are outside `input_sha256`.

`python3 scripts/prepare-calibration-input.py` is the deterministic packet producer. It reads this procedure, the selected manifest/profile policy, and the exact supplied bytes, then writes a fresh isolated directory with source records, `instructions.txt`, the canonical request, and mechanical hashes. It never copies `expected`, `observed`, or oracle fields and never invokes a reviewer. The closed `--coverage-case` selector accepts only `sufficient`, `related-insufficient`, or `implementation-defect`; those packets are external evidence, not manifest rows. `python3 scripts/assert-calibration-capture.py` is the read-only consumer: it joins prepared packets to retained driver capture indexes/receipts, verifies source identities, gate-specific prompt hashes, raw returned output, and pending owner-attestation fields without invoking a model or changing data.

## Canonical digest framing

A11 computes `input_sha256` as lowercase SHA-256 over one binary source-record stream:

1. Prefix stream with big-endian unsigned 64-bit source-record count.
2. For every source record in order, append big-endian unsigned 64-bit label-byte length, exact label bytes, big-endian unsigned 64-bit content-byte length, and exact content bytes.
3. Use no separators and perform no post-hash normalization.

Lengths count bytes, not characters. The instruction, prompt, protocol, template, schema, subject, predecessor, companion, and request records are all included when supplied. Expected, observed, attested_by, model/invocation metadata, row index, fixture outcome, and reviewer output are outside this identity.

Selected schema bytes use the value at `artifact_schemas/{subject}` from selected profile JSON. Recursively sort every object key by bytewise UTF-8 lexicographic order, serialize compact UTF-8 JSON with comma and colon separators, and emit no trailing LF. Do not otherwise parse or normalize supplied bytes.

Any supplied-byte change changes the mechanical identity of each current source stream containing it; unrelated rows remain scoped to their own source set. Keep required profile `config_version` bumps and exact current-row hashes, but neither a bump nor byte drift establishes changed judging behavior. Fresh semantic review is required for affected judging obligations, materiality rules or fixture meaning; otherwise explicit owner/owner-delegated driver applicability may select a genuine retained judgment. `input_sha256` is mechanical identity, not semantic review proof or an applicability classifier. No provider runtime path reads or interprets it.

All historical v10/v11/v12/v13/v14 rows are frozen evidence snapshots. Preserve exact row order, source hash, returned result, model, invocation and attestation identity, or original pending state. Never recompute old hashes from a successor profile or relabel old attestations as fresh. Current source-record hashing and packet preparation use `minimal-15`, `standard-15` and `high-rigor-15` keys and profiles; preparation refuses a historical key against a successor profile. A separate explicit applicability declaration on the current row may reference a genuine historical judgment under A11; it does not change its source or claim that the current packet was reviewed then.

## Canonical request JSON

`request-json` is one UTF-8 JSON object with exactly six string fields in this order:

```json
{"gate":"...","policy_id":"...","review_stage":"...","subject":"...","subject_revision":"...","config_version":"..."}
```

Values are row `gate`, row `axis`, row `review_stage`, gate subject, selected subject fixture `revision`, and selected profile `config_version`. Use RFC 8259 string quoting: escape quote, backslash, and controls; use `\b`, `\t`, `\n`, `\f`, and `\r` for backspace, tab, newline, form feed, and carriage return; use lowercase `\u00xx` for every other U+0000–U+001F. Do not escape slash or non-ASCII characters. Use only comma and colon separators. Emit no insignificant whitespace, duplicate keys, or trailing LF. Supply exact request bytes; parsing then reserializing is not equivalent.

## Input framing locator-line-counts/v1

A manifest row may carry `"input_framing": "locator-line-counts/v1"`. It is defined only for ordinary non-narrative review packets and, today, exactly the nine rows approved for a marker (listed with their unframed hashes in the A11 consumer); every other row has no marker and its source-record stream is unchanged (version 0). Of those nine, one still carries this marker and is pending, and eight carry the v1.1 marker below.

A marked row adds one source record, label `framing:locator-line-counts/v1`, shown to the reviewer as "locator line-count note", placed immediately before `request-json`; `request-json` stays the final record. The record is a deterministic pure function of the ordered records that precede it: a fixed explanation of the line-range locator rule (1-based, inclusive, physical lines of the exact body of the file; the engine's own `lines().count()` rule; JSON-pointer locators unchanged) and one line per citable supplied material and per embedded file naming the neutral locator name and its line count. Names come from the neutral role labels, never from canonical fixture labels. The preparer, the capture asserter and the A11 consumer all derive the same bytes; none accepts a note it cannot re-derive.

`input_sha256` stays the ordinary framed digest over the actual record list, so a marked row's hash differs from the hash of the same row without the marker and no returned judgment can satisfy both. The evidence bodies, the expected answer and every other source record are unchanged. A marked row uses its own attested-invocation literal (`Fresh owner-attested review with input framing locator-line-counts/v1: ...`, defined in the A11 consumer) and cannot use explicit applicability, as source or as target. A judgment returned for a row without the marker is a version 0 judgment and is never relabeled as, or counted with, a version 1 judgment. The preparer takes the framing only through `--input-framing locator-line-counts/v1` and refuses a flag that disagrees with the row's marker.

## Input framing locator-line-counts/v1+locator-syntax/v1.1

A manifest row may carry `"input_framing": "locator-line-counts/v1+locator-syntax/v1.1"`. It has every property of version 1 above, except that the line-count note is no longer the record immediately before `request-json`, and adds one second source record, label `framing:locator-syntax/v1.1`, shown to the reviewer as "locator syntax requirement", placed immediately after the version 1 line-count note and immediately before `request-json`. Unlike the note, this record is fixed text, not derived: the same 725 bytes in every row (defined in the preparer as `FRAMING_V11_RECORD` and in the A11 consumer as `INPUT_FRAMING_V11_RECORD`). It states that a line-range locator must be written `FILE#Lstart-Lend` (`FILE#L3-L7` valid, `FILE#L3-7` invalid), that JSON-pointer locators and the earlier line counts are unchanged, and asks the reviewer to check each locator against that syntax and the line counts before returning. It is not evidence and names no file, criterion or verdict.

The evidence bodies, the expected answer, the reviewer protocol and every other source record are unchanged. `input_sha256` is the ordinary framed digest over the actual record list, so a v1.1 row's hash differs from the hash of the same row with version 1 framing and from the hash without any framing; a judgment returned for one version is never relabeled as, or counted with, another. A v1.1 row uses its own attested-invocation literal (`Fresh owner-attested review with input framing locator-line-counts/v1+locator-syntax/v1.1: ...`, defined in the A11 consumer) and cannot use explicit applicability. The preparer takes it through `--input-framing locator-line-counts/v1+locator-syntax/v1.1`, the capture asserter re-derives the note and compares the fixed record, and the A11 consumer rebuilds both records and the digest.

Eight rows hold a versioned matching judgment under this framing, each retained under the exact input hash of the packet that was judged (original returns preserved in `returned-v17.json`; earlier returns for the same rows made without framing or with unclarified framing stay in the driver's evidence and are not relabeled). Rows whose valid judgment disagrees with the manual oracle are not recorded as observed results: a manifest row is either attested with `observed` equal to `expected` or pending, so those rows stay pending and the final no-pending gate stays unmet for them.

## Recording attestation

After changed PASS/FAIL judging obligations, materiality rules or fixture meaning, leave affected current rows pending until genuine fresh external review and owner inspection/attestation. Other input-byte, profile/version, representation or integration changes require mechanical current identity updates and explicit fresh-or-applicable selection, not a blanket fresh review.

Uncovered current rows retain explicit pending fields:

- `observed`: `pending`.
- `attested_by`: empty string.
- `invocation`: `Fresh review pending: mechanical rehash complete; owner must perform exact fresh review and attest returned evidence before green calibration.` This existing pending literal grants no automatic requirement for fresh review where valid explicit applicability is permitted.

For fresh review, owner inspects the actual returned evidence and records only the exact returned `pass` or `fail`, owner identity in `attested_by`, actual model, exact current `input_sha256`, and the existing invocation literal:

`Fresh owner-attested review: copy exact config example_prompt, reviewer-protocol.md, paired fixture inputs, then request one JSON review-evidence record; no prompt adaptation.`

For unchanged judging behavior, record explicit applicability on the current manifest row, not on the historical source. Use one optional closed `applicability` object with `source_row`, `result_ref` and `reason`, all nonempty strings. `source_row` is the existing stable `config_version|gate|axis|review_stage|fixture_id` key; `result_ref` locates the actual immutable returned prior output; `reason` is the owner/owner-delegated driver's short declaration that judging obligations, materiality and fixture meaning are unchanged and why the source judgment remains applicable. Existing `attested_by` identifies that driver. Set current `observed` and `model` to the exact source returned result and model, keep the current packet's exact `input_sha256`, and use the distinct invocation literal:

`Explicit owner/owner-delegated driver applicability: genuine retained source judgment selected for unchanged judging behavior; not a fresh current-packet review.`

Mechanically resolve exactly one `source_row`, require matching profile family, gate, axis, stage and fixture identity, reject pending/unattested sources, verify the actual returned prior result/model and any recorded row identity agree with that source, and require the selected result to equal the current unchanged expectation. Do not infer applicability from source/current hash equality or inequality, version spelling, protocol-byte drift or this declaration's prose. Semantic applicability is trusted explicit driver judgment under LE-107. Use a genuine judgment source rather than a chain of applicability claims; unavailable, invalid, mismatched or substantively incomplete prior results cannot count. Keep all historical row bytes and their source hashes/output history unchanged. No second inventory, semantic hash classifier or provenance framework is added.

Never change `expected`, row keys, fixture classes or coverage to force agreement. `example-evidence.json` remains illustrative, not an attestation. No current row is populated from its expected oracle, relabeled as a fresh review, or silently treated as unchanged. Changed fixture meaning requires fresh review, not the identity check alone. Real returned results and any already recorded observations are preserved; this procedure invents none.

Final validation retains the existing ignored A11 no-pending gate `calibration_manifest_has_no_pending_rows_for_final_validation`, scoped to the current corpus. Derive the complete required nonempty current shipped-profile/gate/axis/stage/fixture set from **Coverage universe and pairing**, not a second inventory. Fail on empty or missing coverage, pending rows, invalid sources, changed judging meaning without fresh review, or absent owner inspection/explicit applicability. Each required row must select either a genuine fresh judgment with owner inspection/attestation or a genuine retained judgment with explicit applicability and cheap source/result checks. Historical pending rows neither satisfy current proof nor block fully covered current rows. This is not sampling, automatic re-attestation, fresh relabeling, oracle-copying or deletion. No wording amendment alone completes preparation, applicability appraisal, external review or checker correction. An explicit approved numerical call/time/cost budget is required before any paid fresh calibration call; no 396-call blanket refresh budget is approved.

Public scripted operator-path proof covers shipped tool availability, context handling, output formatting and recovery to completed outcomes. It proves those mechanics, not semantic PASS/FAIL calibration. The owner or owner-delegated driver appraises affected judging behavior explicitly; independent exact review of the appraisal is required for this delivery before treating output/integration-only changes as applicable. Do not automatically declare all semantic rows unchanged.

## Reset rule

Reset (change fixtures, expected class, or restart a key) only for:

- wrong verdict;
- leaked expected class;
- materially false finding; or
- a defect that would systematically admit bad work or reject good work.

Wording-only rounds do not reset the corpus.

## Futility and materiality boundary

Good fixture may contain minor blemishes but no material defect affecting named axis. Defective fixture contains concrete material defect and should fail with findings naming why it affects success against intent. Review findings must remain axis-scoped, evidence-based, and materially supported. Generic framing, stylistic weakness, or hypothetical concerns do not justify resetting a row unless they can change verdict, independence, evidence integrity, or realistic workflow/product outcome. Owner decides materiality before changing supplied material or attestation.

## Successor v14 sufficiency coverage

Every shipped intent-sufficient and acceptance-sufficient policy/stage uses its respective sufficiency-good and sufficiency-defective fixture pair (fixture IDs are metadata only). Intent-sufficient includes the cold-singleton stage and aggregate ordinary/challenge stages; acceptance-sufficient includes aggregate ordinary/challenge stages. All use neutral r15 revisions.

After required predecessors and before request-json, intent-sufficient rows receive companion:fictional-repo/implementation-evidence/baseline-2026-08-12.txt. Intent-sufficient challenge rows then receive companion:fictional-repo/implementation-evidence/intent-sufficient-comparison.md. Acceptance-sufficient challenge rows receive companion:fictional-repo/implementation-evidence/acceptance-sufficient-coverage.md. These retained ordinary accounts describe cleared claims, not challenge outcomes.

All v14 rows start pending. All validation-review/validation-adversarial-review rows and new sufficiency rows require fresh judgments. For remaining rows, a separate applicability proposal references the genuine original v12 judgment directly; the proposal is not manifest applicability or authorization. Historical v13 rows remain unchanged.

## Successor v15 parallelism coverage

Historical v14 rows are preserved unchanged. Existing v15 axis rows select the original genuine fresh source judgments by explicit owner/owner-delegated driver applicability when judging meaning is unchanged; never chain applicability through a carried v14 row. Current packet hashes and canonical requests use the -15 identity and exact current supplied bytes. The six `parallelism-honest` fixtures and the `plan-serial-honest` predecessor override above apply at every successor profile, gate and stage that configures the axis. Only the new axis requires fresh external judgments for this change; the v14 sufficiency coverage section records its historical preparation, not a new v15 refresh obligation.

## Successor v15 narrative calibration (preparation is not attestation)

Retain all 1976 historical rows, including 420 v14 rows, in order with their
original bytes, identities, hashes, actual returns, attestations and pending
states. Append 420 existing-key v15 counterparts with unchanged expectations
and new mechanical identities. They remain pending until independent driver
appraisal authorizes direct applicability of an actual original judgment;
never chain applicability through a v14 applicability claim. Fresh v14 sources
must match the actual retained return's result, model and input hash. An external
applicability proposal is not a manifest declaration or attestation.

The current universe additionally derives narrative keys from **both** shipped
`extra.narrative_review` template arrays, substituting each of design, plan and
validation into the ordinary/challenge gate. Include every axis and stage, even
though narratives are off by default. Minimal and standard each have 18 keys;
high-rigor has 36 (individual and aggregate). Each has two rows: 72 keys and 144
fresh narrative rows. With the parallelism-honest rows, current totals are
minimal 156, standard 156, high-rigor 300: 612 rows and 288 keys. No current
pending row satisfies final proof.

The narrative subject is `<phase>-narrative.json` with composite revision
`r15/1`; the approved artifact retains `r15`. The provider-owned
`data/narrative-schema.json` is supplied verbatim, never selected from caller
artifact_schemas. Bindings cite the exact existing approved good phase artifact
and the phase markdown filename and hash. Fixture IDs and classifications are
preparation metadata; reviewer-facing framing uses neutral artifact roles.

Pairings (manual preparation oracle only, not reviewer instructions):

| Phase | Axis | Faithful | Seeded contrast | AC-4 failure kind |
|---|---|---|---|---|
| design | faithful | narrative-101 | narrative-111 | contradiction |
| design | intent-aligned | narrative-101 | narrative-113 | invention/overpromise |
| design | understandable | narrative-101 | narrative-112 | internal inconsistency |
| plan | faithful | narrative-102 | narrative-121 | untraceable claims |
| plan | intent-aligned | narrative-102 | narrative-123 | intent drift |
| plan | understandable | narrative-102 | narrative-122 | material omission |
| validation (minimal) | faithful | narrative-103 | narrative-131 | softened uncertainty |
| validation (standard) | faithful | narrative-103 | narrative-133 | overclaimed delivery |
| validation (high-rigor) | faithful | narrative-103 | narrative-134 | hidden residuals |
| validation (all profiles) | intent-aligned | narrative-103 | narrative-133 | overclaimed delivery |
| validation | understandable | narrative-103 | narrative-132 | unintelligible without Loop context |

Each failure kind is seeded on one natural phase, not copied into every phase.
There are exactly thirteen narrative cases: three faithful narratives and ten
separately seeded contrasts. Validation uncertainty, overclaimed delivery and
hidden residuals each have their own narrative; one FAIL cannot stand for all
three kinds. The uncertainty contrast discloses the residual but dismisses its
significance; the delivery contrast claims an installed outcome while retaining
honest identity/revision limitations; the residual contrast omits those limitations
without assuring delivery or minimizing a disclosed risk. Profile-family selection
only changes the seeded fixture for validation faithful keys, preserving all
ordinary/challenge stages and the two-row pair for every key. Validation
intent-aligned keys use the delivery contrast because it claims excluded scope.
Design internal inconsistency gives opposing explanations of independent author counting;
plan omission removes the dependency, purpose and observable completion links
that a reader needs, rather than just shortening the text. Every faithful
narrative is reviewed on all three axes, at all stages and ordinary/challenge
contexts. These are expectations awaiting independent observation, not verdicts.

### Narrative-only exact source stream and format precedence

The existing-key stream above remains unchanged. For narrative rows use exactly:

1. `system-developer-instruction:data/calibration/reviewer-instruction.txt`
2. `review-worker-preamble:data/review-worker-preamble.txt` (actual shipped bytes)
3. `assignment-json` (compact sorted JSON, one assigned axis/stage, author
   `calibration-reviewer`, supplied-only boundary and explicit format precedence)
4. `example_prompt` (exact template string, without adaptation)
5. `reviewer-protocol:data/reviewer-protocol.md`
6. `template:data/templates/narrative.md`
7. `schema:data/narrative-schema.json` (exact bytes)
8. `subject:data/calibration/fixtures/{fixture_id}.json` (binding bytes)
9. `markdown` (exact selected markdown bytes)
10. `approved-artifact` (exact existing good phase artifact bytes)
11. `frozen-intent` (exact intent-good bytes)
12. Required predecessors: none for design, design-good for plan, design-good,
    plan-good and implementation-report-good for validation, using the existing
    `required predecessor:data/calibration/fixtures/{fixture_id}.json` labels.
13. Validation supplies `companion:validation-evidence.json`, the selected
    repository-state companion, then the four requirement-proof companions in
    their existing specified order. No live fictional path resolution.
14. Challenge only: `challenge-context` with exact phase/axis corpus ordinary
    review and empty cleared finding-ledger bytes. The preparation metadata
    explicitly declares `synthetic-corpus-stimulus-not-genuine-review`: these
    controlled clean-conclusion accounts with concrete grounds/locators are
    **not** actual reviewer returns, ordinary clearance or driver attestation.
15. `request-json` (the unchanged six-field canonical framing, narrative subject
    and `r15/1` revision).

Hash all labels and bytes with the same u64 framing; no second hash inventory.
The assignment explicitly resolves the old calibration instruction's
single-record wording: the **actual shipped preamble** governs narrative output.
Return one raw batch-v3 object with exactly author, review_contract_version,
review_stage and judgments; exactly one assigned-axis fresh judgment with its
actual result, findings and grounds. Do not return validation_verdicts for
narrative review. No formatter may manufacture gate/result/reason fields or
silently substitute a single-record protocol. Preserve raw batch output and
select the actual matching judgment at intake.

The producer refuses in-checkout output and replacement packet directories,
and emits dispatch inventories naming row/input identity, instructions and exact
source paths. Graders receive only supplied instructions/material, not manifest,
procedure, preparation metadata, dispatch identities or this pairing oracle.
`assert-calibration-capture.py --preparation-only` verifies packets without
returns; it establishes no semantic judgment. Narrative captures are checked
against actual shipped current sources and the assigned raw batch. Unsupported
oracle metadata, altered bytes, wrong format or mismatched axes are rejected.

Filled fresh current rows require `result_ref` to an immutable retained JSON
entry with actual row_key, model, input_sha256 and unmodified raw output. A11
checks those identities and the actual assigned result; pending rows have no
result_ref. Historical return files remain untouched. Store newly commissioned
returns only after genuine calls; an empty driver-owned preparation returns file
is explicitly pending and is not evidence. Parent independently reviews the
reuse proposal, commissions fresh narrative judgments after packet inspection,
inspects actual returns and records owner-delegated attestation. The ignored
no-pending gate remains required and must reject this pending preparation.
