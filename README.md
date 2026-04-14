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

8つのシグナルを重み付きで合算します。

| シグナル | 重み | 役割 |
|----------|------|------|
| ドメイン類似度 | 22% | ファイルの「機能的識別子」を比較。汎用ファイル名（index等）は親ディレクトリをドメインとして使用し、具体的ファイル名（IndexController等）はタイプワードを除いたトークンをドメインとする |
| プライマリステム類似度 | 26% | ファイル名の本体（最初の`.`より前）同士を比較。複合拡張子を正しく扱う核心 |
| 拡張子除去後のファイル名類似度 | 20% | すべての拡張子を除いた実ファイル名を比較し、`UserService.ts` と `UserService.spec.ts` のような対応を強く拾う |
| Jaccard係数（トークンベース） | 15% | 階層が異なっても共通ドメイン名を持つファイルを検出 |
| ディレクトリ近接度 | 10% | 共通ディレクトリ階層が多いほど高スコア |
| 拡張子類似度 | 2% | `.ts` や `.spec.ts` などの一致を弱い補助シグナルとして扱う |
| Jaro-Winkler距離 | 3% | プレフィックスが共通なパスを軽く優遇 |
| Levenshtein類似度 | 2% | パス全体の編集距離ベース比較 |
| 最終更新日時ボーナス | 最大+0.1 | 最近更新されたファイルをわずかに優遇 |

さらに以下の補正を加算します。

- **サブストリングブースト（+0.15）** — ステム類似度計算時、一方のステムが他方に含まれる場合（例: `index` ⊂ `indexcontroller`）
- **コンテンツ類似度ボーナス（最大+0.1）** — パス類似度が0.9以上の候補に対して、ファイル内容の単語トークンJaccard類似度を追加
- **頻出ファイル名ペナルティ** — プロジェクト内で同じベース名が多いほど、`style` や `index` のような識別力の低い名前として自動的に減衰させる
- **同階層コンテキスト優先** — 起点ファイル名が頻出名なら、別ディレクトリの同名ファイルより同じディレクトリの関連ファイルを優先しやすくする

### ドメイントークンの抽出

ドメイン類似度シグナルの核心となる「ドメイントークン」は以下のルールで決定します。

| ケース | ルール | 例 |
|--------|--------|-----|
| 汎用ファイル名（index, show, create…） | 親ディレクトリ名をドメインとする | `search/index.phtml` → `search` |
| 具体的ファイル名（CamelCase等） | タイプワード（Controller, Model等）を除いたトークン | `IndexController.php` → `index` |

これにより、`view/search/index.phtml`（ドメイン: "search"）と`view/index/index.phtml`（ドメイン: "index"）が別ドメインと識別され、`IndexController.php`（ドメイン: "index"）が同ドメインとして正しく上位に来ます。

### ファイル参照の解析

#### 引用符スキャン（全言語共通）

ファイル内の引用符（`"..."` `'...'`）をスキャンし、以下のスタイルの内部パス参照を抽出します。

| スタイル | 例 | 対象 |
|---------|-----|------|
| スラッシュ | `/application/search` | URL・ファイルパス全般 |
| バックスラッシュ | `App\Http\Controllers\HomeController` | PHP名前空間・Windowsパス |
| ドット記法 | `detail.index`, `home.create` | Laravelビュー名・ZF2ルート名など |

外部URL（http/https/mailto等）は自動除外されます。

#### 言語別・非引用符スキャン

ファイル拡張子から言語を検出し、引用符なしのimport/use文を追加解析します。

| 言語 | 対象パターン | ノイズ除去 |
|------|-------------|----------|
| Python (`.py`) | `import pkg.mod`、`from pkg.mod import X` | — |
| Rust (`.rs`) | `use crate::module::Item;`、`mod name;` | `crate`、`std`、`super` 等 |
| Java/Kotlin (`.java`/`.kt`) | `import com.example.Class;` | `com`、`org`、`java`、`javax` 等 |
| C# (`.cs`) | `using Company.Product.Class;` | `System`、`Microsoft` 等 |
| PHP (`.php`/`.phtml`) | `use App\Models\User;` | フレームワーク名（`illuminate` 等） |

`src`・`app`・`lib`・`resources` など、多くのファイルに共通して現れるディレクトリ名はJaccard計算時の重みを0.2に下げ、スコアが引っ張られないようにします。

### ファイル収集

[`ignore`](https://docs.rs/ignore) クレートを使用し、`.gitignore` に記載されたファイル・ディレクトリを自動で除外します。

## 利用クレート

| クレート | 用途 |
|----------|------|
| [`clap`](https://docs.rs/clap) | CLIの引数解析 |
| [`ignore`](https://docs.rs/ignore) | `.gitignore`を考慮したファイル再帰検索 |
| [`strsim`](https://docs.rs/strsim) | Levenshtein距離・Jaro-Winkler距離の計算 |
