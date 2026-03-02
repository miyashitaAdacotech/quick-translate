# Quick Translate (Rust版)

Alfred の Quick Translate にインスパイアされた Windows 用翻訳ツール。Python版をRustでリライト。

## セットアップ

### 1. Rust のインストール

```powershell
# winget でインストール（推奨）
winget install Rustlang.Rustup

# または https://rustup.rs/ からインストーラーをダウンロード
```

### 2. ビルド

```powershell
cd quick-translate-rs
cargo build --release
```

ビルド成功すると `target/release/quick-translate.exe` が生成される。

### 3. 実行

```powershell
# ポップアップを表示
cargo run --release

# テキストを直接翻訳
cargo run --release -- --translate "Hello World"

# ヘルプ
cargo run --release -- --help
```

## 操作方法

| キー | 動作 |
|------|------|
| 文字入力 | リアルタイム翻訳（400ms デバウンス） |
| `Ctrl+Enter` | 翻訳結果をクリップボードにコピーして閉じる |
| `Esc` | 閉じる |

## 設定ファイル

初回起動時に `~/.quick-translate/config.json` が自動生成される。

```json
{
  "engine": "google",
  "deepl_api_key": "",
  "source_lang": "auto",
  "target_lang_ja": "en",
  "target_lang_en": "ja",
  "font_size": 16.0,
  "opacity": 0.95,
  "log_enabled": true,
  "hotkey_popup": "ctrl+shift+t",
  "hotkey_selected": "ctrl+shift+y"
}
```

## プロジェクト構成

```
src/
├── main.rs          # エントリポイント、CLI引数パース
├── config.rs        # 設定ファイル読み書き (serde)
├── lang.rs          # 言語自動判定 (Unicode範囲チェック)
├── translator.rs    # Google翻訳エンジン (reqwest)
└── popup.rs         # egui ポップアップUI
```

## 開発ロードマップ

- [x] フェーズ1: コアMVP（CLI + ポップアップ + Google翻訳）
- [ ] フェーズ2: システムトレイ常駐 + グローバルホットキー
- [ ] フェーズ3: DeepL対応 + 翻訳ログ + スタートアップ登録

## テスト

```powershell
cargo test
```
