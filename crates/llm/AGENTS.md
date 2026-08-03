Prefer golden tests for complete request/response translation and wire serialization changes.
Avoid a new fixture for one trivial field; extend a representative fixture such as
`full.json` when practical. Keep focused unit tests for errors, invariants, and
streaming or state-machine behavior.
