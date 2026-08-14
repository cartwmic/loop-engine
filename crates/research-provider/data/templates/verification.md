# Verification artifact

Write `verification.json` as adversarial claim checking against gathered sources. The provider does not judge claim truth.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when claims or challenges are materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `sources_revision`: must equal current `sources.json` revision when the `verified` gate is checked.
- `claims`: one or more records with:
  - `id`: stable identifier later citations can use
  - `statement`: the claim being checked
  - `source_ids`: ids from `sources.json`
  - `support`: how cited extracts support the claim
  - `challenge`: a genuine contrary extract or an explicit record that none was found after search

Verification-local corrections stay in verify: edit and recheck `verification.json`, then retry checked `verified`. Use nearest check-free `revise` for sources-owned defects or `revise-brief` for brief-owned defects. Do not waive known defects.
