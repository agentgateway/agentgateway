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

if ! PID="$(cat "$process_file")"; then
  echo "unable to read process.txt. was your process started with start.sh?"
  exit 1
fi
touch "$restart_file"
kill "$PID"
