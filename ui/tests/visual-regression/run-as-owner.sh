#!/bin/sh
set -eu

# Read ownership inside the container so rootless user mappings also work.
visual_owner=$(stat -c '%u:%g' /visual)
mkdir -p /visual-cache/npm /visual-cache/pnpm /tmp/visual-home
for visual_directory in /visual-cache/npm /visual-cache/pnpm /visual/reference/ui/node_modules /visual/current/ui/node_modules /tmp/visual-home; do
	if [ "$(stat -c '%u:%g' "$visual_directory")" != "$visual_owner" ]; then
		chown -R --no-dereference "$visual_owner" "$visual_directory"
	fi
done
exec setpriv --reuid="${visual_owner%:*}" --regid="${visual_owner#*:}" --clear-groups \
	env HOME=/tmp/visual-home "$@"
