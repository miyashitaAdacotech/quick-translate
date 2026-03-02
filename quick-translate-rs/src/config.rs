// config.rs - 設定ファイルの読み書き
//
// ~/.quick-translate/config.json にJSON形式で設定を保存する。
// ファイルが存在しない場合はデフォルト値で自動生成する。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// アプリケーション設定
///
/// `#[derive(Serialize, Deserialize)]` を付けると、
/// serde が自動的にJSONとの変換コードを生成してくれる。
/// `#[serde(default)]` を付けると、JSONに存在しないフィールドは
/// Default トレイトのデフォルト値が使われる。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 翻訳エンジン: "google" または "deepl"
    pub engine: String,

    /// DeepL API キー（空文字列 = 未設定）
    pub deepl_api_key: String,

    /// ソース言語: "auto" で自動判定
    pub source_lang: String,

    /// 日本語テキストの翻訳先
    pub target_lang_ja: String,

    /// 英語テキストの翻訳先
    pub target_lang_en: String,

    /// フォントサイズ
    pub font_size: f32,

    /// ウィンドウの透明度 (0.0 - 1.0)
    pub opacity: f32,

    /// 翻訳ログを有効にするか
    pub log_enabled: bool,

    /// ポップアップ起動のホットキー
    pub hotkey_popup: String,

    /// 選択テキスト翻訳のホットキー
    pub hotkey_selected: String,
}

/// Default トレイトの実装
/// `Config::default()` で呼ばれるデフォルト値を定義する。
/// Python版の DEFAULT_CONFIG と同じ値。
impl Default for Config {
    fn default() -> Self {
        Self {
            engine: "google".to_string(),
            deepl_api_key: String::new(),
            source_lang: "auto".to_string(),
            target_lang_ja: "en".to_string(),
            target_lang_en: "ja".to_string(),
            font_size: 16.0,
            opacity: 0.95,
            log_enabled: true,
            hotkey_popup: "ctrl+shift+t".to_string(),
            hotkey_selected: "ctrl+shift+y".to_string(),
        }
    }
}

/// 設定ディレクトリのパスを返す
/// Windows: C:\Users\<ユーザー名>\.quick-translate
fn config_dir() -> PathBuf {
    // dirs::home_dir() でホームディレクトリを取得
    // .expect() は None の場合にパニック（クラッシュ）する
    dirs::home_dir()
        .expect("ホームディレクトリが見つかりません")
        .join(".quick-translate")
}

/// 設定ファイルのパスを返す
fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// 設定ファイルを読み込む
///
/// ファイルが存在しない場合はデフォルト設定を生成して保存する。
/// JSONにないフィールドは `#[serde(default)]` によりデフォルト値が使われる。
pub fn load_config() -> Config {
    let path = config_file();

    if path.exists() {
        // ファイルを読み込む
        // match式: Result型の成功(Ok)と失敗(Err)を分岐する
        match fs::read_to_string(&path) {
            Ok(contents) => {
                // JSONをパース
                match serde_json::from_str(&contents) {
                    Ok(config) => return config,
                    Err(e) => {
                        eprintln!("設定ファイルのパースに失敗: {}", e);
                        // パース失敗時はデフォルト値を使う
                    }
                }
            }
            Err(e) => {
                eprintln!("設定ファイルの読み込みに失敗: {}", e);
            }
        }
    }

    // ファイルがない or 読み込み失敗 → デフォルトを保存して返す
    let config = Config::default();
    let _ = save_config(&config); // エラーは無視（初回起動時にディレクトリがないことがある）
    config
}

/// 設定ファイルを保存する
pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let dir = config_dir();
    // ディレクトリを再帰的に作成（存在する場合はOK）
    fs::create_dir_all(&dir)?;

    let path = config_file();
    // serde_json::to_string_pretty でインデント付きJSONに変換
    let json = serde_json::to_string_pretty(config)?;
    fs::write(&path, json)?;

    Ok(())
}
