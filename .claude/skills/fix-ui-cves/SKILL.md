---
name: fix-ui-cves
description: Audit and remediate security vulnerabilities in a frontend package using multiple scanners: `yarn npm audit`, `trivy`, and `grype`.
---

# Fix UI CVEs

Audit and remediate security vulnerabilities in a frontend package using multiple scanners: `yarn npm audit`, `trivy`, and `grype`.

## Steps

1. **Identify the target UI package directory.**

   Run the following to find all `package.json` files in the repo, excluding `node_modules`:

   ```
   find . -name "package.json" -not -path "*/node_modules/*"
   ```

   - If **exactly one** `package.json` is found, use its directory as `<UI_DIR>` and proceed automatically.
   - If **more than one** is found, present the list to the user and ask them to choose which directory to audit. Wait for their response before proceeding. Use the chosen directory as `<UI_DIR>` for all subsequent steps.

2. **Ensure the correct Node version** is active before proceeding. Run the following **in parallel**:
   - Read `.nvmrc` at the root of the repo to get the required version
   - Run `node --version` to get the current version

   If the versions match, proceed to step 3. If not:
   - Try `fnm`: `eval "$(fnm env)" && fnm use`
   - If `fnm` is not available, try `nvm`: `export NVM_DIR="$HOME/.nvm" && [ -s "$NVM_DIR/nvm.sh" ] && source "$NVM_DIR/nvm.sh" && nvm use`
   - If neither tool is available and the Node version is wrong, **stop and report the error** — do not proceed with the wrong Node version.

3. **Ensure all scanners are installed, then run them all in parallel** and collect findings:

   First, check that trivy and grype are installed (these checks can run in parallel with each other):

   **trivy** — check with `which trivy`:
   - If missing, install: `brew install aquasecurity/trivy/trivy`
   - If installed, check the version (`trivy --version`) and update if it is outdated: `brew upgrade trivy`

   **grype** — check with `which grype`; if missing, install via Homebrew cask (the formula is deprecated):

   ```
   brew install --cask grype
   ```

   Once all scanners are confirmed installed, **update the grype vulnerability database** before scanning (this avoids repeated DB downloads during the scan itself):

   ```
   grype db update
   ```

   Then **run all three in parallel using sub-agents** (dispatch three simultaneous Agent tool calls, one per scanner):
   - **Sub-agent 1:** `cd <UI_DIR> && yarn npm audit` — return the full output
   - **Sub-agent 2:** `trivy fs <UI_DIR> --scanners vuln --vuln-type library` — return the full output
   - **Sub-agent 3:** `GRYPE_DB_AUTO_UPDATE=false grype dir:<UI_DIR> 2>&1 | grep -v "go-module"` — return the full output

   Wait for all three sub-agents to complete before proceeding.

   **Note on trivy false positives:** trivy will also scan any `yarn.lock` files nested inside `node_modules/` (e.g. `node_modules/better-opn/yarn.lock`). These are lockfile artifacts bundled inside sub-dependencies and do not reflect packages installed in this project. Findings under `node_modules/**/yarn.lock` are false positives and should be ignored — only findings from the top-level `yarn.lock` are actionable.

   **Note on grype go-module findings:** grype scans compiled Go binaries inside `node_modules/` (e.g. `node_modules/@esbuild/darwin-arm64/bin/esbuild`) and reports CVEs in the embedded Go standard library. These are **not JavaScript package vulnerabilities** — they cannot be fixed via yarn package management and are out of scope for this skill. Filter them out with `grep -v "go-module"` as shown above.

   After running all available scanners, **consolidate the findings** into a single deduplicated list of vulnerable packages and CVE IDs before proceeding to remediation. If no vulnerabilities are found across all scanners, report that and stop.

4. **For each vulnerability found**, try in order:

   a. **Direct upgrade first** — bump the affected package to a non-vulnerable version in `<UI_DIR>/package.json` dependencies or devDependencies, then run `yarn install` inside `<UI_DIR>/`.

   b. **If a direct upgrade isn't possible** (e.g. the affected package is a transitive dependency), add a `resolutions` entry to `<UI_DIR>/package.json`:

   ```json
   "resolutions": {
     "vulnerable-package": ">=safe-version"
   }
   ```

   **Before using `>=safe-version`, check whether any dependency declares a peer dep range that caps the major version of this package.** For example, if storybook declares `"vite": "^5.0.0 || ^6.0.0 || ^7.0.0"`, then `"vite": ">=7.0.8"` would pull in v8 and break that peer dep. In such cases, cap the resolution: `">=7.0.8 <8.0.0"`.

   Then run `yarn install` inside `<UI_DIR>/`.

   **After `yarn install`, check for any new `YN0060` warnings** that were not present before. A new warning involving a package you just resolved almost always means the resolution jumped a major version that some dependency's peer dep range doesn't cover — add a `<next-major` cap to the resolution and re-run `yarn install`.

5. **Re-run all available scanners in parallel using sub-agents** (same three sub-agents as step 3, with `GRYPE_DB_AUTO_UPDATE=false` since the DB was already updated) to confirm the vulnerability is resolved across every tool. If issues remain in any scanner, repeat step 4 with a different approach.

6. **Check for peer dependency misalignment** after any upgrade. When upgrading a package that is part of a suite (e.g. `storybook`, `eslint`, `babel`, `jest`), scan `package.json` for other packages in the same suite that may now declare mismatched peer dependencies against the newly upgraded version:
   - Look for sibling packages (e.g. `@storybook/*` when upgrading `storybook`) still pinned to an older major.
   - Run `yarn install` and look for `YN0060` peer dependency warnings involving the upgraded package.
   - For each misaligned package:
     - Check if the suite now bundles the API into the root package (e.g. `storybook/manager-api` in v9 replaces `@storybook/manager-api`). If so, update all import paths and remove the now-redundant direct dependency.
     - If a matching version exists in the registry, upgrade the sibling to match.
     - If no compatible version is available and the API is unchanged, document the mismatch clearly but do not force an incompatible version.

7. **Prune unnecessary resolutions.** For each resolution added during this session, check whether it is still required:

   a. Remove the resolution entry from `package.json`.
   b. Run `yarn install` inside `<UI_DIR>/`.
   c. Check the resolved version in `yarn.lock` (e.g. `grep -A2 '"package@npm' yarn.lock | grep version`).
   d. If the lockfile still resolves to a safe version without the resolution, the resolution is no longer needed — leave it removed.
   e. If the lockfile drops back to a vulnerable version, restore the resolution entry.

   Repeat for each resolution. This keeps `package.json` clean and avoids stale overrides that can prevent natural future upgrades.

8. **Run a build** to verify nothing is broken after pruning:

   ```
   cd <UI_DIR> && yarn build
   ```

9. **Report** what was changed: which packages were upgraded directly, which required resolutions, any peer dependency realignments or import path migrations, which scanners were run (and which were skipped), and confirm the final results from each scanner are clean.

## Notes

- Always prefer direct upgrades over resolutions — resolutions are a last resort.
- When adding resolutions, use `>=safe-version` rather than pinning to an exact version unless there is a specific reason to pin.
- If a vulnerability has no fix available yet, report it clearly and skip it rather than leaving a broken build.
- The project uses `fnm` (preferred) or `nvm` for Node version management and `yarn` (v4) as the package manager.
