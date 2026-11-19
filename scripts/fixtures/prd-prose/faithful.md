# Notes app: product requirements

This document says what the notes app must do. It was accepted by the owner on 2026-11-01.

> Requirements index: NA-1 saving, NA-2 unsaved mark, NA-3 search, NA-4 no upload, NA-5 export, NA-6 export order, NA-7 to NA-9 limits.
> The prose below is the accepted text and wins over this index.

## Saving

The app saves every note to disk within one second of the last keystroke.
- Requirement: NA-1
- Status: live

A note that cannot be saved is shown with a red mark, and the app keeps the text in memory until the save works.
- Requirement: NA-2
- Status: live

## Search

Search finds a note by any word in its title or body. Results appear as the user types.
- Requirement: NA-3
- Status: live

The app must not send note text to any server. This is a promise to users.
- Requirement: NA-4
- Status: live

## Export

A user can export all notes as one markdown file. The file lists notes oldest first:

```
# Export

## Note title
Note text.

## Next note title
More text.
```

- Requirement: NA-5, NA-6
- Status: live

Export is the only way to move notes to another computer.

## Limits

The app enforces these limits:
- A note title is at most 100 characters.
  - Requirement: NA-7
  - Status: live
- A note body is at most 1 MB.
  - Requirement: NA-8
  - Status: live
- A user has at most 10000 notes.
  - Requirement: NA-9
  - Status: live

## Open questions

Should deleted notes be kept for 30 days? The owner has not decided.
- Source question, not a requirement. Returned to the owner.
