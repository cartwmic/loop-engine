# Sources artifact (generate-PRD)

Write `sources.json` as the gathered source record. Perform search and fetch externally; the provider never retrieves URLs.

Gather from the current repository. Each source locator must be a reviewable repository path, test, document, or extract that later verification can open in this checkout.

Required machine-checked shape:

- `revision`: non-empty author-declared revision. Bump when the source set is materially different.
- `author`: `{name, kind}`, where `kind` is `human`, `agent`, or `script`.
- `brief_revision`: must equal current `brief.json` revision when the `gathered` gate is checked.
- `sources`: one or more records with:
  - `id`: stable identifier later claims can cite
  - `title`: source title
  - `locator`: repository path or other locator
  - `extract`: quoted or closely paraphrased extract sufficient for later claim checks

Cite extracts verification can check. Use check-free `revise` when a brief-owned defect must return to scope.
