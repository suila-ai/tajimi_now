# tajimi_now

気象庁の公開データを Misskey に投稿し続ける bot。

- アメダスの観測値（気温・風速・日照・降水量）を 10 分ごとに投稿する。発表中の警報・注意報があれば、レベルごとに色を付けて添える。
- 日付が変わった後、前日の最高・最低気温 → その日の天気予報 → 00:00 の観測値の順に投稿する。前日の気温と天気予報はピン留めし、前日のものと差し替える。天気予報は前日 17 時の発表から作る。

警報・注意報は、2026 年 5 月 29 日からの新しい防災気象情報（注意報・警報・危険警報・特別警報の 4 段階）に対応している。氾濫（旧 洪水）は河川ごとの発表で市区町村には結び付かないので扱わない。

## 使い方

Docker と Docker Compose を使う。

1. `config.example.toml` を `config.toml` に写し、Misskey のサーバとアクセストークン（`read:account`・`write:account`・`write:notes`）、気象庁の各コードを書く。コードの引き方はファイル内のコメントにある。bot はコンテナの中で uid 65532 として動くので、`config.toml` はその uid から読める権限にしておく（自分だけが読める 600 にすると、Permission denied で起動に失敗し続ける）。
2. `docker compose up -d --build` で起動する。ログは `docker compose logs -f` で見る。
3. 設定を変えたら `docker compose restart` で読み直す。コードを更新したら `docker compose up -d --build` で作り直す。

Misskey は 2025.10.0 から 2026.5.0 までのサーバを相手にできる。

## 開発

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo test -- --ignored live_jma      # 気象庁の実データに対する取得
```

ローカルの Misskey に向けて動かすには、`scripts/misskey/up.sh` で使い捨てのサーバを立て、表示される手順に従う。`scripts/misskey/down.sh` でデータごと消える。

気象庁のデータの出典は、それぞれのモジュール（`src/jma/`）の冒頭に書いてある。
