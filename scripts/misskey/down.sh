#!/usr/bin/env bash
# 開発用の Misskey を、データと .env.local ごと消す。
set -euo pipefail
cd "$(dirname "$0")"
docker compose -p tajimi-now-misskey -f compose.yml down -v
rm -f .env.local
