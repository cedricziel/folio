#!/usr/bin/env bash
# Usage: with-instance.sh <command> [args...]
# Starts an ephemeral instance, runs the command with FOLIO_VERIFY_ENV (env file),
# BASE_URL and VERIFY_DIR exported, then always tears the instance down.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
env_file=$("$here/up.sh")
trap '"$here/down.sh" "$env_file"' EXIT

# shellcheck disable=SC1090
source "$env_file"
export FOLIO_VERIFY_ENV=$env_file BASE_URL VERIFY_DIR
"$@"
