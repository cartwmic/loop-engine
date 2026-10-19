# Owner brief: task-board label keys

The public request deliberately does not specify the exact matching rules or the
fallback. Answer questions about a rule with that rule only; do not volunteer
other rules. No CLI, dependencies, persistence or new feature is needed. Preserve
the public function `label_key(label: str) -> str`, editing only `labels.py`.
Inputs are strings; non-string input behavior is outside this case.

## REQ-1: Whitespace normalization

Remove leading and trailing whitespace and collapse every internal run of Unicode
whitespace to a single ASCII space. Preserve non-whitespace punctuation. Reveal
this when asked how spaces/whitespace should affect grouping.

## REQ-2: Unicode caseless grouping (hidden)

Keys must use Unicode casefolding, not merely lowercase. For example, `Straße`
and `STRASSE` must both produce `strasse`. Reveal this when asked about casing,
language/Unicode support, or exact text matching rules. This requirement is not
stated in the public request or starting repository.

## REQ-3: Empty-label fallback (hidden)

An empty or whitespace-only label must have the literal key `inbox`, not an empty
key and not an error. Reveal this when asked about blank/empty labels or fallback
behavior. The particular fallback value is only in this hidden brief.
