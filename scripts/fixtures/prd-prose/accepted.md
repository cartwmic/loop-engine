# Notes app: product requirements

This document says what the notes app must do. It was accepted by the owner on 2026-11-01.

## Saving

The app saves every note to disk within one second of the last keystroke.

A note that cannot be saved is shown with a red mark, and the app keeps the text in memory until the save works.

## Search

Search finds a note by any word in its title or body. Results appear as the user types.

The app must not send note text to any server. This is a promise to users.

## Export

A user can export all notes as one markdown file. The file lists notes oldest first:

```
# Export

## Note title
Note text.

## Next note title
More text.
```

Export is the only way to move notes to another computer.

## Limits

The app enforces these limits:
- A note title is at most 100 characters.
- A note body is at most 1 MB.
- A user has at most 10000 notes.

## Open questions

Should deleted notes be kept for 30 days? The owner has not decided.
