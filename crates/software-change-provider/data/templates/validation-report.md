# Validation report

Report proof that delivered change meets intent and repository obligations.

Required metadata:

- non-empty `revision` and `author`;
- `intent_revision` matching current `intent.json`;
- `coverage.commit`: one repository commit identity for covered repository state;
- `coverage.documents`: list of covered documents, each with `path` and declared `revision`.

Record `outcome`, `requirements` (requirement-to-proof entries), and `validation` commands/results. Include documentation integration proof for authoritative documents. A completed policy-document audit run over affected documents is acceptable evidence; composition with document-audit workflow happens at orchestration level, never through provider coupling.
