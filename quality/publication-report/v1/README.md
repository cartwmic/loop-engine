# Publication report v1

This directory freezes closed JSON Schemas for semantic evaluation, owner approval, and aggregate publication-attempt evidence. Runtime records never live here.

Records use compact canonical UTF-8 JSON emitted by `serde_json` from closed typed structures. Object fields follow schema order; maps use lexicographically sorted keys; no trailing newline is written. External IDs are lowercase SHA-256 over exact stored bytes and are not embedded in those bytes.

Runtime records live below `/usr/bin/git rev-parse --git-common-dir` at `loop-engine/validation/v1`. Writes use create-new temporary files, file synchronization, and an atomic no-overwrite link into the digest path. Reads verify path digest, exact bytes, canonical encoding, closed shape, nullability, derived disposition, and candidate/config/rubric bindings.
