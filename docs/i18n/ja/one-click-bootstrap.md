# ローカライズブリッジ: One Click Bootstrap

このページは強化版ブリッジです。テーマの位置付け、原文セクション導線、実行時の注意点をまとめています。

英語版原文:

- [../../one-click-bootstrap.md](../../one-click-bootstrap.md)

## テーマ位置付け

- 分類: ランタイムと接続
- 深度: 強化ブリッジ（セクション導線 + 実行ヒント）
- 使い方: 構成を把握してから、英語版の規範記述に従って実施します。

## 原文ガイド

- 実際のセクション移動は英語版原文の見出しを基準にしてください。
- ブリッジ本文と英語版の見出し構成に差分がある場合は、英語版原文を優先します。

## 実行ヒント

- 既存インストールの更新は、まず `topclaw update --check`、次に `topclaw update`、サービス運用中なら `topclaw service restart` の順で行います。
- ホスト型 one-line installer は、まず最新 release の互換バイナリを優先し、ソースビルドへのフォールバックが必要な場合だけリポジトリを clone します。ローカル変更の検証には checkout 上で `./bootstrap.sh --force-source-build` を使います。
- まず原文の見出し構成を確認し、今回の変更範囲に直結する節から読みます。
- コマンド名、設定キー、API パス、コード識別子は英語のまま保持します。
- 仕様解釈に差分が出る場合は英語版原文を優先します。

## 関連エントリ

- [README.md](README.md)
- [SUMMARY.md](SUMMARY.md)
- [docs-inventory.md](docs-inventory.md)
