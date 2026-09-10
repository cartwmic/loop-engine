# Current-plan implementation report proof

`scripts/assert-implementation-report.py` checks a report against the supplied
plan-owned matrix and retained execution receipts. It does not run the matrix,
review semantic quality, checkpoint the report, or advance a workflow. There is
no built-in task inventory. The matrix remains a read-only specification, not a
success ledger.

```sh
python3 scripts/assert-implementation-report.py --self-test
python3 scripts/assert-implementation-report.py \
  --report "$AR/implementation-report.json" \
  --revision "$REVISION" --plan-revision 5 \
  --matrix "$AR/proof-matrix.json"
```

The second command exits 0 only when every `local_final` row has current,
completed, passing command evidence. Missing evidence, failed commands, timeout,
spawn errors, missing streams or required stdout markers, and pending local work
exit 1, even if the report claims success. The checker does not require its own
success as an input. Capture its stdout/stderr/exit externally, then perform the
matrix's remaining `post_report` operations. Its self-test is a different,
non-circular local obligation.

## Report claims

Keep the provider's existing report schema. `coverage.commit` is exactly current
`git rev-parse HEAD` plus `+uncommitted-worktree`. `changed_surface` is exactly the
ordered pathname list after each two-letter status from
`git status --porcelain=v1 --untracked-files=all` (including Git's quoting/rename
notation). This is not a sorted set.

`validation` uses the provider's closed `{proof, criterion_id?}` objects, not
strings. Rows without `criterion_id` contain one matrix status in `proof`: all
`local_final` rows in matrix order, then `after_separate_authorization` rows in
matrix order:

```json
[
  {"proof": "cache-version: passed"},
  {"proof": "cache-start: pending"},
  {"proof": "commit-and-push: pending"}
]
```

Optional `{criterion_id: "AC-1", proof: "..."}` rows retain criterion evidence
notes. They do not count as matrix receipts or replace required status rows;
this checker does not judge their semantic sufficiency.

This abbreviated example is not a complete matrix. Local statuses are `passed`,
`failed`, or `pending`; claims must agree with the receipts. Honest failed or
pending local work is valid to *report*, but cannot pass the final checker.
A provisional summarizer report is not final proof. Do not copy obsolete T06
commands, terminal-run markers, packaged-smoke obligations, or hosted success
strings into a new report merely to satisfy the old checker.

`post_report` rows are not in `validation`. After-authorization rows must remain
`pending` in this pre-authorization implementation report. They are not mandatory
local executions, nor can a local receipt establish them. Retain actual later
Git/hosted/dogfood results separately after their required authorization and
execution; this checker does not approve them. Other context, bootstrap
exceptions, semantic gates and residuals belong in the report's normal summary
and remaining fields, not fabricated command statuses.

## Receipt index

T14 retains `$AR/proof/receipts.json` beside the matrix (the matrix's parent is
`AR`). No matrix modification or additional execution service is needed:

```json
{
  "plan_revision": "5",
  "target_directory": "/absolute/actual/cargo-target",
  "commands": [
    {"id": "cache-version", "receipt": "cache-version/receipt.json"}
  ]
}
```

Resolve Cargo's actual `target_directory` before the build, not by assuming
`target/`. The index names selected receipts only; it does not repeat command
policy. Missing IDs mean pending. Duplicate, unknown and non-local IDs refuse.
Retain failed/earlier attempts and select the appropriate current receipt; never
rewrite a failure into a success. The complete current matrix, not this example,
determines required coverage.

The [operational UX contract](operational-ux-contracts.md#capture-t03) maps common
capture attempts to these receipts. Actual child argv and repository identity
remain mandatory; native provider fan-out evidence is a separate linkage, not
an interchangeable receipt. Preparation does not fabricate either format.

Each receipt uses ordinary subprocess capture facts:

```json
{
  "id": "cache-version",
  "argv": ["sccache", "--version"],
  "cwd": "/absolute/checkout",
  "started_at": 1780000000.0,
  "finished_at": 1780000000.1,
  "wall_seconds": 0.1,
  "exit_code": 0,
  "timed_out": false,
  "spawn_error": null,
  "stdout": "stdout",
  "stderr": "stderr",
  "repository_before": "sha256:MECHANICALLY_COLLECTED",
  "repository_after": "sha256:MECHANICALLY_COLLECTED"
}
```

Receipt references are absolute or relative to the index; stream references are
absolute or relative to the receipt. Both streams must exist, including empty
stderr. Preserve raw bytes. `started_at`/`finished_at` are wall-clock epoch
seconds; `wall_seconds` is elapsed monotonic time. A signal/nonzero exit is not a
pass; timeout and spawn failure remain explicit. A missing exit is not success.
Selected local receipts must be serialized in matrix order.

Collect repository identity immediately before and after execution with the
shared deterministic helper (from the checkout):

```python
import sys
from pathlib import Path
sys.path.insert(0, "scripts")
from test_contract import repository_proof_identity
identity = repository_proof_identity(Path.cwd())
```

It hashes HEAD, Git index/status, and actual tracked/nonignored-untracked content
and modes, without a driver-authored file inventory. It is a local receipt
fingerprint, not the provider checkpoint digest. Ignored build outputs are not
source identity. Both identities must equal the current tree. This detects
same-path edits that HEAD plus a changed-path list alone misses. Do not backfill
current identities into old captures. Keep proof artifacts outside the checkout.
Unsupported repository entry types fail closed.

`argv` must equal matrix `command` plus `args`, replacing only the declared
`{artifact_root}`, `{checkout}`, `{target_directory}`, and
`{implementation_revision}` placeholders. Preserve shell argv as declared rather
than replacing a `bash -c` command with the inner string. `cwd` is the checker
checkout. Required `expected_stdout_contains` strings are checked in the actual
stdout capture, never in report prose. T14 still inspects full artifacts,
scenario coverage, wall time, and descendant cleanup; markers are not semantic
proof or permission to omit the matrix's evidence rules.

All final benchmark collection/comparison rows are ordinary mandatory local
rows. Missing serial, parallel, or comparison receipts/streams fail; nonzero
comparison fails even alongside a narrative speedup claim. The benchmark
harness owns dataset completeness/comparability and improvement decisions. The
checker uses its actual command exit and retained output, not a second copy of
benchmark policy.

## Separate delivery pointer

After review, preserve the implementation report and its complete native
implementation checkpoint (including `repository.entries`), or select that exact
checkpoint from retained `implementation-proof-history`. Do not assemble a
changed-path inventory or substitute the current checkout. The checkpoint's
complete path/content-digest/Git-mode entries reconstruct reviewed content
identity, including deletions; historical HEAD/index/status are not equivalence
criteria. Missing reconstruction or a report/checkpoint digest mismatch refuses.

```sh
python3 scripts/delivery-pointer.py \
  --report /absolute/reviewed/implementation-report.json \
  --checkpoint /absolute/reviewed/implementation-checkpoint.json \
  --run-reference /absolute/run-reference.json \
  --repository /absolute/repository \
  --output /absolute/delivery.json
```

This records Git and hosted facts as pending. Only after separately authorized
Git delivery, add `--commit FULL_SHA`; every committed path, content digest,
executable bit and symlink must match, not just changed files. Metadata-only
commit differences are accepted. No Git action is authorized or executed by
this utility. Run-reference is a retained nonempty JSON object identifying the
reviewed run; the utility preserves its locator/digest, not semantic approval.

Optional `--hosted-evidence FILE` accepts a JSON object with `status` equal to
`pending`, `success` or `failure`. Observed outcomes require the matched `commit`
and a nonempty `url`; these are supplied observations, not a remote verification
or semantic judgment. Omitted hosted facts stay pending even after matching Git.
Repeated use of the same output path creates `.2`, `.3`, etc. without replacing
old records, and links the previous generation by locator/digest. Reviewed
identities cannot change within that sequence. Use one serial driver per output.
Neither terminal records nor pre-commit reports are rewritten.

## Focused public proof

`--self-test` drives this same public CLI with a temporary report/matrix and
actual tiny scripted subprocess receipts, including an actual exit-7 comparison.
It checks positive current evidence, revision/path-order errors, missing
measurements/captures, failure/pending status, stale tree/argv/cwd, missing stdout
markers, serialization, and circular/hosted claims. These synthetic benchmark
names test evidence plumbing, not real speedup.

To retain fixtures and each CLI result outside the checkout:

```sh
python3 scripts/assert-implementation-report.py --self-test \
  --self-test-output /absolute/new/checker-proof-directory
```

The full stable-tree local matrix, document audits/requirement acceptance, final
benchmark measurements, and real workflow review gates remain driver-owned.
