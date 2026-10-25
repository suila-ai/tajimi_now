#!/usr/bin/env bash
# 開発用の使い捨て Misskey を立て、bot 用のアカウントを作って接続情報を .env.local に書く。
# 2 回目以降は、.env.local があればサーバを立てるだけにする。
# docker・curl・jq が要る。
set -euo pipefail
cd "$(dirname "$0")"

compose=(docker compose -p tajimi-now-misskey -f compose.yml)
base=http://localhost:3000

# api <token|""> <endpoint> [json]: 応答のボディを出力する。エラー応答なら失敗する。
api() {
    local token=$1 endpoint=$2 body=${3:-'{}'}
    if [ -n "$token" ]; then body=$(jq -c --arg i "$token" '. + {i: $i}' <<<"$body"); fi
    curl --fail-with-body -sS -m 60 -H 'Content-Type: application/json' -d "$body" "$base/api/$endpoint"
}

"${compose[@]}" up -d --quiet-pull
echo "Misskey の起動を待っている" >&2
for _ in $(seq 150); do
    if api "" meta > /dev/null 2>&1; then break; fi
    sleep 2
done
api "" meta > /dev/null

if [ -f .env.local ]; then
    echo "既存の .env.local を使う" >&2
else
    # 最初のアカウントは認証なしで作れて、管理者になる。bot のアカウントは管理者が作る。
    admin=$(api "" admin/accounts/create '{"username":"admin","password":"password"}' | jq -r .token)
    # 続けて叩いてもレートリミットに掛からないよう、既定ロールの係数を下げる
    api "$admin" admin/roles/update-default-policies '{"policies":{"rateLimitFactor":0.3}}' > /dev/null
    bot=$(api "$admin" admin/accounts/create '{"username":"bot","password":"password"}' | jq -r .token)
    cat > .env.local <<ENV
MISSKEY_SERVER=$base
MISSKEY_TOKEN=$bot
# 管理者のトークン。bot のノートが他のユーザーに見えないことの確認などに使う（ユーザー名 admin、パスワード password）
MISSKEY_ADMIN_TOKEN=$admin
ENV
    echo "アカウントを作り、接続情報を .env.local に書いた" >&2
fi

cat <<USAGE
Misskey 2026.3.2 が $base で動いている。
bot を動かすには、config.example.toml を config.toml に写し、[misskey] の server と token に scripts/misskey/.env.local の MISSKEY_SERVER と MISSKEY_TOKEN の値を書いてから、次を実行する。
  cargo run
Misskey の往復のテストは、接続情報を環境に読み込んでから実行する。
  set -a; . scripts/misskey/.env.local; set +a
  cargo test -- --ignored live_round_trip
止めるときは scripts/misskey/down.sh（データも消える）。
USAGE
