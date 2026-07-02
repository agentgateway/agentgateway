#!/bin/sh
#
# A helper script to restart a given process as part of a Live Update.
#
# Further reading:
# https://docs.tilt.dev/live_update_reference.html#restarting-your-process
#
# Usage:
#   Copy start.sh and restart.sh to your container working dir.
#
#   Make your container entrypoint:
#   ./start.sh path-to-binary [args]
#
#   To restart the container:
#   ./restart.sh

set -eu

state_dir="/tmp/agentgateway-live-update"
process_file="$state_dir/process.txt"
restart_file="$state_dir/restart.txt"
process_id=""

trap quit TERM INT

quit() {
  if [ -n "$process_id" ]; then
    kill "$process_id"
  fi
}

mkdir -p "$state_dir"

while true; do
    rm -f "$restart_file"

    "$@" &
    process_id=$!
    echo "$process_id" > "$process_file"
    set +e
    wait "$process_id"
    EXIT_CODE=$?
    set -e
    if [ ! -f "$restart_file" ]; then
        echo "Exiting with code $EXIT_CODE"
        exit $EXIT_CODE
    fi
    echo "Restarting"
done
