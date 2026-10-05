"""Grouping keys for task-board labels."""


def label_key(label: str) -> str:
    return " ".join(label.split()).casefold() or "inbox"
