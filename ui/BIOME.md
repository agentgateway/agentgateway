# UI formatting and linting

The UI uses [biome](https://biomejs.dev/) for formatting and linting. Its
[configuration](./biome.jsonc) covers source files, tests, scripts, and root configuration files.
Generated TypeScript declarations are left to `pnpm generate-schema`.

[Recommended rules](https://biomejs.dev/linter/#recommended-rules) run with the
[React, Playwright, test, project, and type domains](https://biomejs.dev/linter/domains/). All
enabled rules are errors. Existing findings for `noDescendingSpecificity` and
`noNonNullAssertion` are suppressed inline so new ones fail CI.

Imports within `src` use the `@/` alias, and `noRestrictedImports` rejects relative imports there.
Formatting uses tabs, LF line endings, a 100-character line width, single quotes in JavaScript,
and double quotes in JSX. The biome version in [package.json](./package.json) matches the schema
version in `biome.jsonc`. See the [formatter documentation](https://biomejs.dev/formatter/) for the
full set of formatting options.

## Commands

Run these from `ui`:

```sh
pnpm lint          # Type-check, lint, and check formatting
pnpm check         # Run biome without the TypeScript check
pnpm check:fix     # Apply safe fixes and formatting
pnpm format:check  # Check formatting only
pnpm format        # Apply formatting only
```

Review the diff after either write command. CI runs `pnpm lint`, which includes the TypeScript
check.

## Adding a rule

Enable the rule as an error and apply its safe fixes. Suppress existing findings only when the fix
needs its own behavior review. Record those exceptions in both tables below. `pnpm lint` must
finish without diagnostics before the change is ready.

## Suppressions

Fix findings that are mechanical and preserve current behavior. Keep an inline suppression when a
fix could change React timing, accessibility behavior, list reconciliation, the CSS cascade, or
another user-facing contract.

Every exception uses the same comment:

```ts
// biome-ignore lint/suspicious/noArrayIndexKey: See ui/BIOME.md for why this exception exists and how to remove it.
```

Biome's [suppression documentation](https://biomejs.dev/analyzer/suppressions/) covers placement
and syntax.

There are 123 comments covering 136 diagnostics. Without those comments, all 136 would be errors.
A dependency array can produce several `useExhaustiveDependencies` diagnostics, which is why the
totals differ.

### By rule

| Rule | Diagnostics | Comments | Why it stays and how to remove it |
| --- | ---: | ---: | --- |
| `noLabelWithoutControl` | 5 | 5 | Shared fields pass controls through `children`, while YAML headings use label styling without an editable control. Replace the shared field contract with explicit `id` and `htmlFor` links, and use non-label text for read-only headings. |
| `noNoninteractiveTabindex` | 3 | 3 | Help icons and the log badge receive focus so their tooltips work without a pointer. Replace them with native interactive elements or another tooltip trigger that keeps keyboard access. |
| `noStaticElementInteractions` | 5 | 5 | Composite widgets and dialog backdrops observe events owned by their interactive children or supply an extra pointer close path. Move each interaction to a native control without dropping keyboard behavior. |
| `useKeyWithClickEvents` | 3 | 3 | The combobox handles keys through `aria-activedescendant`. The startup dialog uses its controls for keyboard dismissal. Remove these after the components use native interaction patterns. |
| `useSemanticElements` | 2 | 2 | The segmented control uses buttons with radio state. The clickable log row must remain valid table markup. Replace them only when the native structure can preserve the current layout and keyboard behavior. |
| `useExhaustiveDependencies` | 26 | 13 | These hooks use serialized values or selected fields as intentional triggers. Adding every referenced function or object can change request cadence, reset timing, or routing behavior. Characterize each hook before changing its dependencies, then use stable callbacks or smaller effects. |
| `noDangerouslySetInnerHtml` | 1 | 1 | Log Markdown is sanitized by DOMPurify with an explicit tag allowlist and no allowed attributes. Remove this when Markdown renders through React nodes instead of HTML injection. |
| `noArrayIndexKey` | 37 | 37 | These collections do not have stable UI identities. Add stable identifiers and browser coverage for insertion, deletion, and reordering before changing editable lists. Visual snapshot coverage is enough for display-only lists when it catches layout and ordering regressions. |
| `noDescendingSpecificity` | 50 | 50 | Moving selectors in the shared stylesheet can change the cascade. Rework selector order and specificity with visual regression coverage. |
| `noNonNullAssertion` | 4 | 4 | These assertions rely on nearby runtime checks that TypeScript cannot carry into the expression. Replace them with explicit narrowing without adding fallback behavior that hides broken invariants. |
| **Total** | **136** | **123** | |

### By file

This tally accounts for every inline comment. Run `rg -n "biome-ignore" ui/src` from the repository
root to list the exact locations.

| File | Rule | Comments |
| --- | --- | ---: |
| `src/components/Primitives.tsx` | `lint/a11y/noLabelWithoutControl` | 1 |
| `src/components/Primitives.tsx` | `lint/a11y/noNoninteractiveTabindex` | 2 |
| `src/components/Primitives.tsx` | `lint/a11y/noStaticElementInteractions` | 4 |
| `src/components/Primitives.tsx` | `lint/a11y/useKeyWithClickEvents` | 1 |
| `src/components/Primitives.tsx` | `lint/a11y/useSemanticElements` | 1 |
| `src/components/Primitives.tsx` | `lint/suspicious/noArrayIndexKey` | 3 |
| `src/components/Shell.tsx` | `lint/correctness/useExhaustiveDependencies` | 1 |
| `src/main.tsx` | `lint/style/noNonNullAssertion` | 1 |
| `src/pages/ClientSetup.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/pages/Costs.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/pages/DumpPolicies.tsx` | `lint/a11y/noLabelWithoutControl` | 1 |
| `src/pages/Guardrails.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/pages/Home.tsx` | `lint/a11y/noStaticElementInteractions` | 1 |
| `src/pages/Home.tsx` | `lint/a11y/useKeyWithClickEvents` | 2 |
| `src/pages/Keys.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/pages/Logs.tsx` | `lint/a11y/noNoninteractiveTabindex` | 1 |
| `src/pages/Logs.tsx` | `lint/a11y/useSemanticElements` | 1 |
| `src/pages/Logs.tsx` | `lint/correctness/useExhaustiveDependencies` | 9 |
| `src/pages/Logs.tsx` | `lint/security/noDangerouslySetInnerHtml` | 1 |
| `src/pages/Logs.tsx` | `lint/style/noNonNullAssertion` | 3 |
| `src/pages/Logs.tsx` | `lint/suspicious/noArrayIndexKey` | 7 |
| `src/pages/McpPlayground.tsx` | `lint/correctness/useExhaustiveDependencies` | 1 |
| `src/pages/McpPlayground.tsx` | `lint/suspicious/noArrayIndexKey` | 1 |
| `src/pages/Models.tsx` | `lint/suspicious/noArrayIndexKey` | 4 |
| `src/pages/Playground.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/pages/Policies.tsx` | `lint/correctness/useExhaustiveDependencies` | 1 |
| `src/pages/TrafficGateways.tsx` | `lint/suspicious/noArrayIndexKey` | 1 |
| `src/pages/TrafficListeners.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/pages/TrafficRoutes.tsx` | `lint/suspicious/noArrayIndexKey` | 3 |
| `src/pages/models/ModelMatchesEditor.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/pages/models/ProviderConfigEditor.tsx` | `lint/correctness/useExhaustiveDependencies` | 1 |
| `src/pages/traffic/TrafficConfigDumpPanel.tsx` | `lint/a11y/noLabelWithoutControl` | 3 |
| `src/policies/AuthorizationPolicyEditor.tsx` | `lint/suspicious/noArrayIndexKey` | 1 |
| `src/policies/McpGuardrailsPolicyEditor.tsx` | `lint/suspicious/noArrayIndexKey` | 1 |
| `src/policies/RemoteRateLimitPolicyEditor.tsx` | `lint/suspicious/noArrayIndexKey` | 2 |
| `src/styles.css` | `lint/style/noDescendingSpecificity` | 50 |
| **Total** | | **123** |
