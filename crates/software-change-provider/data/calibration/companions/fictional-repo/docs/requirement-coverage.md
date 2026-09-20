# Requirement coverage fixture

This fictional document is the authoritative cross-reference named by the
requirement used in the coverage examples. It is supplied material for
reviewers; it is not the repository's live PRD.

### REQ-COVERAGE-1: Operator-visible job status
- Status: accepted

After an operator starts a job, the public status view must show whether the
job is `running`, `succeeded`, `failed`, or `unknown`. Process exit by itself
is not acceptance: the result must still be checked against the declared
output and acceptance conditions.

The requirement applies to the local operator status path. It does not require
proactive owner chat, a push service, or a new notification channel.
