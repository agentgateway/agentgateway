# Repository contribution instructions

All GitHub PR descriptions, issues, and comments MUST include the :spaghetti: emoji.

## Build counter policy

`api/data.gen.go` contains repository build metadata. For every change to code,
increment `BuildCounter` by exactly one.

This counter update is an intentional part of the requested change, even when the
code being edited is elsewhere in the repository.
