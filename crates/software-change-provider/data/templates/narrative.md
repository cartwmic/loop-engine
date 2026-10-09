# Writing a phase narrative

Write only when the run asks for the narrative, after the phase's JSON artifact
has cleared ordinary and challenge review. Read that exact approved artifact and
the frozen intent. The JSON stays authoritative for later phases; do not edit it
as part of narrative writing. If writing exposes a JSON defect, use the phase's
revise route, then write a fresh narrative after the revised artifact clears review.

Write plain-English markdown for the owner and an engineer without Loop Engine
context. Explain necessary specialist terms and workflow concepts rather than
assuming the reader knows gate names, record IDs or tools. Say what is proposed or
delivered, why it meets the intent, how its parts fit together and what limits it.
Use headings and source references where they help the reader follow the meaning.
There is no prescribed length, readability score or one-section-per-JSON-field rule.

- **Design:** Explain the resulting solution, important boundaries, consequential
  choices and their reasons, coverage of the intended outcomes, and material risks.
- **Plan:** Explain the work to deliver that design, dependencies, task ownership,
  and how completion and required proof will be observed. Do not describe planned
  work as already delivered.
- **Validation:** Explain what was actually delivered and what the retained proof
  establishes. Distinguish successful, failed, missing and inapplicable checks;
  make remaining limitations and accepted risks visible. Do not overclaim delivery
  or hide residuals, even if the report reached a terminal workflow state.

Compare every material claim with the approved artifact or its cited evidence.
Avoid contradiction of the artifact, material omission, invention or overpromise,
softened uncertainty, drift from the frozen intent, untraceable claims and internal
inconsistency. Preserve qualifications and uncertainty that change the reader's
understanding. Summarizing is welcome; dropping material meaning is not. Review
also asks whether the narrative is understandable to an engineer without Loop
Engine context. For validation, review additionally rejects overclaimed delivery
and hidden residuals. Review does not invent requirements or assess share safety.

## Files and binding

Keep a separate pair for each enabled phase under the run's artifact directory:

| Phase | Approved artifact | Narrative markdown | Review subject (binding) |
|---|---|---|---|
| design | design.json | design-narrative.md | design-narrative.json |
| plan | plan.json | plan-narrative.md | plan-narrative.json |
| validation | validation-report.json | validation-narrative.md | validation-narrative.json |

The binding's closed shape is in
`crates/software-change-provider/data/narrative-schema.json`. For example (replace
the example hashes with SHA-256 of the exact artifact and markdown bytes):

```json
{
  "revision": "3/1",
  "author": {"name": "narrative writer", "kind": "agent"},
  "artifact": {
    "subject": "design.json",
    "revision": "3",
    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  },
  "approved_artifact_record_id": "approved-design-3",
  "markdown": {
    "file": "design-narrative.md",
    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  }
}
```

Cite the latest approved-artifact context record for the described artifact, not
an earlier approval or a record invented to fill the field. The bounded schema
checks closed shape; the provider's binding validation additionally checks the
revision prefix, positive decimal counter without leading zeroes, matching phase
file name and 64-character lowercase hexadecimal hashes. File and context-record
freshness are separate gate checks, not established by shape validation.

The revision is `<exact artifact revision>/<narrative counter>`. Every rewrite
uses a new counter and fresh ordinary and challenge narrative review. A revision
is single-use once admitted, even if you restore old prose. A new artifact
revision needs a new prefix, a new binding and fresh narrative review. Do not
change admitted markdown under an existing revision. The provider never rewrites
the markdown. Later phases use their own files and never replace earlier narratives.
