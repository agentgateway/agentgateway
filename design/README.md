# Design Documents

Design documents capture proposed changes that need more context than a GitHub issue or pull request can comfortably
hold. They are point-in-time proposals, so the current implementation may differ after review or implementation.

## Naming

Use the following filename format:

```text
<number>-<kebab-case-title>.md
```

The number should be the related issue number. An issue must exist before adding a design document. For early
drafts without a stable number, use `XXXX` and rename the file before merge.

Examples:

- `288-inferencepool-ai-policies.md`
- `XXXX-new-routing-feature.md`

## Template

Start new design documents from [template.md](template.md). Keep the sections that help explain the proposal and remove
sections that do not apply.

At minimum, each design document should include:

- A title that starts with `EP-<number>`.
- Links to related issues or pull requests.
- A status.
- A date in `M/D/YYYY` format.
- The standard note that the design may become outdated as implementation evolves.
- A summary, goals, non-goals, design details, and test plan.
