"""Diagnostics for the lean-change arms, read from the final engine snapshot and history. They never decide success."""
from collections import Counter

PRINCIPLES_QUESTION = "principles"


def _latest(records, kind):
    rows = [r for r in records if r["kind"] == kind]
    return rows[-1] if rows else None


def lean_metrics(snapshot, history):
    """Counts of denials (by code), criteria, verifications and review findings (by question) with their dispositions."""
    context = snapshot["context"]
    transitions = [h["action"] for h in history if h["action"]["kind"] == "transition"]
    denied = [t["outcome"]["feedback"].get("code", "unknown") for t in transitions if t["outcome"].get("outcome") == "denied"]
    intent = _latest(context, "intent")
    reviews = [r for r in context if r["kind"] == "review"]
    first = reviews[0]["data"] if reviews else None
    findings = first["findings"] if first else []
    dispositions = [r["data"] for r in context if r["kind"] == "finding-disposition"]
    first_id = reviews[0]["id"] if reviews else None
    disposed = {d["finding"]: d["disposition"] for d in dispositions if d["review"] == first_id}
    return {
        "lifecycle": snapshot["lifecycle"], "final_state": snapshot["current_state"],
        "review_enabled": (snapshot.get("initial_input") or {}).get("review", True),
        "transitions": len(transitions), "denials": len(denied), "denials_by_code": dict(Counter(denied)),
        "records_by_kind": dict(Counter(r["kind"] for r in context)),
        "intent_revisions": sum(1 for r in context if r["kind"] == "intent"),
        "hard": bool(intent and intent["data"].get("hard")),
        "criteria": len(intent["data"].get("criteria", [])) if intent else 0,
        "verifications": sum(1 for r in context if r["kind"] == "verification"),
        "reviews_appended": len(reviews),
        "review_fresh_context": first.get("fresh_context") if first else None,
        "review_findings": len(findings),
        "review_findings_by_question": dict(Counter(f["question"] for f in findings)),
        "principles_findings": sum(1 for f in findings if f["question"] == PRINCIPLES_QUESTION),
        "findings_fixed": sum(1 for f in findings if disposed.get(f["id"]) == "fixed"),
        "findings_rejected": sum(1 for f in findings if disposed.get(f["id"]) == "rejected"),
        "findings_undisposed": sum(1 for f in findings if f["id"] not in disposed),
    }
