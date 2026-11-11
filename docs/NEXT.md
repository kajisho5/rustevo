# 引き継ぎメモ（2026-11-11）

別のセッションが続きをやるためのメモ。設計の正は `docs/DESIGN.md`、その冒頭の「実装して変わった点」。

## 今の状態

- v0.1 の中核は main にある。`./scripts/check.sh` が通る（GitHub の自動実行で x86 と ARM の両方を確認済み）。
- デモ `apps/pomodoro` は 12 回の試み（合格 10、不合格 2）。
- 公開台帳ページは**設計とデザインまで**（ブランチ `ledger-site-design`、`docs/LEDGER-SITE.md`）。Shogo さんの確認待ちで、実装していない。

## 残っている作業（おすすめの順）

1. **公開台帳ページ**: `docs/LEDGER-SITE.md` の「確認してほしい点」に答えをもらってから実装する。
2. **`actions/checkout` を v7 に上げる**（今は v4）。
3. **ソースからロジックを作り直して保存分と一致するかの検証**（`evo verify --rebuild`）。今は保存された wasm の動きしか確かめていない。
4. **式の型検査をコンパイル時に**。今は実行時だけ。
5. **スクリーンショット**（ドライバの 8 つ目の操作）。描画を入れる必要がある。合否には使わない。
6. **台帳の署名**。今はハッシュの連鎖だけ。
7. GitHub クライアントのデモ（通信の依頼。v0.2）。

## 世代を足すときの手順

```
cargo build --release -p evo-cli
E=./target/release/evo A=apps/pomodoro
# 1. 先に change.json を書く（intent と、変わるはずの fields / derives / nodes / effects）
# 2. app.evo と logic/src/lib.rs を直す
$E build $A
# 3. 新しい動きがあれば記録を取る
$E run $A <名前>; $E tap $A <名前> <id>; $E clock $A <名前> +1000
$E carry $A        # 下見
$E seal $A --by "<だれが>"
$E verify $A
```

- 不合格でも台帳に残る。直して `seal` し直すと次の番号になる。
- 不合格のあと `pending/` の記録は古いビルドのものなので、消して取り直す。
- `generations/` と `ledger.jsonl` は手で書き換えない（検証が失敗する）。

## 決まりごと

- コミットの作者メールは info@seventh-well.com。
- 報告とドキュメントは分かりやすい日本語。「版」ではなく「バージョン」。
- 見本のデータに実在の名前を使わない。
- 画面を持つものは、設計書 → SVG デザイン → 確認 → 実装の順。
