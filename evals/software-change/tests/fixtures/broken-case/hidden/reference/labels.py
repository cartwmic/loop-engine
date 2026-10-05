"""Grouping keys for task-board labels."""


def label_key(label: str) -> str:
    return " ".join(label.split()).lower() or "inbox"
