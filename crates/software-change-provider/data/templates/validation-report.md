# Validation report

Report proof that the delivered change meets the frozen intent, operating boundary, and repository obligations.

Before writing the report, inspect the current `intent.json`, especially `operating_context`, `accepted_risks`, and `outside_obligations`. An accepted risk cannot waive a stated outcome or outside obligation.

Required metadata:

- non-empty `revision` and `author`;
- `intent_revision` matching current `intent.json`;
- `coverage.commit`: one repository commit identity for covered repository state;
- `coverage.documents`: list of covered documents, each with `path` and declared `revision`.

Record `outcome` as an observable result, not a completion claim. In `requirements`, map every requirement and acceptance criterion to proof of the public user or operator outcome. Name the scenario, the command or procedure, and the observable assertion; activity-only evidence such as a changed-file list, internal test, or passing command without its outcome does not prove delivery. For every new or changed Bookends citation, inspect the cited public scenario and observable assertion semantically: a matching requirement token alone is not proof. Include documentation integration proof for authoritative documents. A completed policy-document audit run over affected documents is acceptable evidence; composition with document-audit workflow happens at orchestration level, never through provider coupling.
