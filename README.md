# akin

指定したファイルに関連するファイルを、プロジェクト内から自動で探索するCLIツールです。

単純なルールベースのマッピングではなく、複数の文字列類似度アルゴリズムを組み合わせることで、プロジェクト構造の変更に強い柔軟な探索を実現します。

```
$ akin src/controllers/UserController.ts

1.0000  src/controllers/UserController.spec.ts
0.8754  src/controllers/PostController.ts
0.7169  tests/UserController.test.ts
0.5977  src/views/UserView.ts
```

## インストール

```bash
cargo install --path .
```

## 使い方

```bash
akin <TARGET> [OPTIONS]
```

### 引数

| 引数 | 説明 |
|------|------|
| `<TARGET>` | 関連ファイルを探したいファイルのパス |

### オプション

| オプション | デフォルト | 説明 |
|------------|-----------|------|
| `-n, --top <N>` | `10` | 表示する結果の件数 |
| `-t, --threshold <SCORE>` | `0.3` | 表示するスコアの最小値（0.0〜1.0） |

### 例

```bash
# 上位10件（デフォルト）
akin src/controllers/UserController.ts

# 上位5件に絞る
akin src/controllers/UserController.ts -n 5

# 類似度スコア0.5以上のみ表示
akin src/controllers/UserController.ts -t 0.5
```

## 仕組み

### 前処理（正規化）

比較前にパス文字列を以下の手順で正規化します。

1. **全拡張子の除去** — `.spec.ts` や `.test.js` のような複合拡張子もすべて除去
2. **小文字化** — 大文字小文字の揺れを吸収
3. **トークン化** — `/`・`\`・`.`・キャメルケース・スネークケースの区切りで分割
   - 例: `src/UserController` → `["src", "user", "controller"]`

### スコアリング

7つのシグナルを重み付きで合算します。

| シグナル | 重み | 役割 |
|----------|------|------|
| ドメイン類似度 | 25% | ファイルの「機能的識別子」を比較。汎用ファイル名（index等）は親ディレクトリをドメインとして使用し、具体的ファイル名（IndexController等）はタイプワードを除いたトークンをドメインとする |
| プライマリステム類似度 | 25% | ファイル名の本体（最初の`.`より前）同士を比較。複合拡張子を正しく扱う核心 |
| Jaccard係数（トークンベース） | 20% | 階層が異なっても共通ドメイン名を持つファイルを検出 |
| ディレクトリ近接度 | 15% | 共通ディレクトリ階層が多いほど高スコア |
| Jaro-Winkler距離 | 10% | プレフィックスが共通なパスを優遇 |
| Levenshtein類似度 | 5% | パス全体の編集距離ベース比較 |
| 最終更新日時ボーナス | 最大+0.1 | 最近更新されたファイルをわずかに優遇 |

さらに以下の補正を加算します。

- **サブストリングブースト（+0.15）** — ステム類似度計算時、一方のステムが他方に含まれる場合（例: `index` ⊂ `indexcontroller`）
- **コンテンツ類似度ボーナス（最大+0.1）** — パス類似度が0.9以上の候補に対して、ファイル内容の単語トークンJaccard類似度を追加

### ドメイントークンの抽出

ドメイン類似度シグナルの核心となる「ドメイントークン」は以下のルールで決定します。

| ケース | ルール | 例 |
|--------|--------|-----|
| 汎用ファイル名（index, show, create…） | 親ディレクトリ名をドメインとする | `search/index.phtml` → `search` |
| 具体的ファイル名（CamelCase等） | タイプワード（Controller, Model等）を除いたトークン | `IndexController.php` → `index` |

これにより、`view/search/index.phtml`（ドメイン: "search"）と`view/index/index.phtml`（ドメイン: "index"）が別ドメインと識別され、`IndexController.php`（ドメイン: "index"）が同ドメインとして正しく上位に来ます。

### ノイズトークンの重み調整

`src`・`app`・`lib`・`resources` など、多くのファイルに共通して現れるディレクトリ名はJaccard計算時の重みを0.2に下げ、スコアが引っ張られないようにします。

### ファイル収集

[`ignore`](https://docs.rs/ignore) クレートを使用し、`.gitignore` に記載されたファイル・ディレクトリを自動で除外します。

## 利用クレート

| クレート | 用途 |
|----------|------|
| [`clap`](https://docs.rs/clap) | CLIの引数解析 |
| [`ignore`](https://docs.rs/ignore) | `.gitignore`を考慮したファイル再帰検索 |
| [`strsim`](https://docs.rs/strsim) | Levenshtein距離・Jaro-Winkler距離の計算 |
