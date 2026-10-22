#!/usr/bin/env bash

echo "|Field|Type|Description|"
echo "|-|-|-|"
# Arguments after the schema file are passed to jq (e.g. --argjson shared, --arg root).
jq -r "${@:2}" -f "$( dirname -- "${BASH_SOURCE[0]}" )"/schema_paths.jq "$1" | sed 's|\.\[\]\.|[].|g'
