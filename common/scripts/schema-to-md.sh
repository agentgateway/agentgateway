#!/bin/bash

echo "|Field|Column|"
echo "|-|-|"
jq -r -f "$( dirname -- "${BASH_SOURCE[0]}" )"/schema_paths.jq "$1"| sed 's|.\[\].|\[\].|g'
