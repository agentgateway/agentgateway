# Fix UI CVEs

Audit and remediate security vulnerabilities in the `ui` package using `yarn npm audit`.

## Steps

1. **Ensure the correct Node version** is active before proceeding:
   - Check if the current Node version matches the project requirement (from `.nvmrc` at the root of the repo).
   - If the version is already correct, proceed to step 2.
   - If not, try `fnm`: `eval "$(fnm env)" && fnm use`
   - If `fnm` is not available, try `nvm`: `export NVM_DIR="$HOME/.nvm" && [ -s "$NVM_DIR/nvm.sh" ] && source "$NVM_DIR/nvm.sh" && nvm use`
   - If neither tool is available and the Node version is wrong, **stop and report the error** — do not proceed with the wrong Node version.

2. **Run the audit** from the `ui` directory:

   ```
   cd ui && yarn npm audit
   ```

   If there are no vulnerabilities, report that and stop.

3. **For each vulnerability found**, try in order:

   a. **Direct upgrade first** — bump the affected package to a non-vulnerable version in `ui/package.json` dependencies or devDependencies, then run `yarn install` inside `ui/`.

   b. **If a direct upgrade isn't possible** (e.g. the affected package is a transitive dependency), add a `resolutions` entry to `ui/package.json`:

   ```json
   "resolutions": {
     "vulnerable-package": ">=safe-version"
   }
   ```

   Then run `yarn install` inside `ui/`.

4. **Re-run the audit** to confirm the vulnerability is resolved. If issues remain, repeat step 3 with a different approach.

5. **Check for peer dependency misalignment** after any upgrade. When upgrading a package that is part of a suite (e.g. `storybook`, `eslint`, `babel`, `jest`), scan `package.json` for other packages in the same suite that may now declare mismatched peer dependencies against the newly upgraded version:
   - Look for sibling packages (e.g. `@storybook/*` when upgrading `storybook`) still pinned to an older major.
   - Run `yarn install` and look for `YN0060` peer dependency warnings involving the upgraded package.
   - For each misaligned package:
     - Check if the suite now bundles the API into the root package (e.g. `storybook/manager-api` in v9 replaces `@storybook/manager-api`). If so, update all import paths and remove the now-redundant direct dependency.
     - If a matching version exists in the registry, upgrade the sibling to match.
     - If no compatible version is available and the API is unchanged, document the mismatch clearly but do not force an incompatible version.

6. **Run a build** to verify nothing is broken:

   ```
   cd ui && yarn build
   ```

7. **Report** what was changed: which packages were upgraded directly, which required resolutions, any peer dependency realignments or import path migrations, and confirm the final audit is clean.

## Notes

- Always prefer direct upgrades over resolutions — resolutions are a last resort.
- When adding resolutions, use `>=safe-version` rather than pinning to an exact version unless there is a specific reason to pin.
- If a vulnerability has no fix available yet, report it clearly and skip it rather than leaving a broken build.
- The project uses `fnm` (preferred) or `nvm` for Node version management and `yarn` (v4) as the package manager.
