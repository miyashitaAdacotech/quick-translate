// translator.rs - 翻訳エンジン
//
// Google翻訳の無料エンドポイントを使ってテキストを翻訳する。
// Python版の deep-translator ライブラリと同じAPIを直接叩く。
// 将来的にDeepL APIにも対応予定。

use crate::config::Config;
use crate::lang::detect_target_lang;

/// 翻訳結果を格納する構造体
#[derive(Debug, Clone)]
pub struct TranslationResult {
    /// 翻訳されたテキスト
    pub translated: String,
    /// 翻訳先言語コード (例: "en", "ja")
    /// フェーズ2でUI表示に使用予定
    #[allow(dead_code)]
    pub target_lang: String,
}

/// Google翻訳の無料APIを使って翻訳する
///
/// エンドポイント: https://translate.googleapis.com/translate_a/single
/// これは deep-translator や他の無料翻訳ライブラリが内部で使っているAPIと同じ。
///
/// # 引数
/// - `text`: 翻訳するテキスト
/// - `target`: 翻訳先言語コード (例: "en", "ja")
/// - `source`: ソース言語コード ("auto" で自動判定)
///
/// # 戻り値
/// - `Ok(String)`: 翻訳結果テキスト
/// - `Err(...)`: HTTPエラーやパースエラー
fn google_translate(
    text: &str,
    target: &str,
    source: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // reqwest::blocking::Client はHTTPクライアント
    // 本当はアプリ全体で使い回すべきだが、シンプルさ優先でここで作る
    let client = reqwest::blocking::Client::new();

    // Google翻訳APIにGETリクエストを送る
    // クエリパラメータ:
    //   client=gtx  : 無料版クライアント識別子
    //   sl=auto     : ソース言語（autoで自動判定）
    //   tl=ja       : ターゲット言語
    //   dt=t        : 翻訳テキストを返す
    //   q=Hello     : 翻訳するテキスト
    let response = client
        .get("https://translate.googleapis.com/translate_a/single")
        .query(&[
            ("client", "gtx"),
            ("sl", source),
            ("tl", target),
            ("dt", "t"),
            ("q", text),
        ])
        .send()?;

    // レスポンスをJSONとしてパース
    // Google翻訳APIのレスポンス形式:
    // [[[翻訳テキスト, 原文, ...], ...], ...]
    // → ネストされた配列の [0][0][0] が翻訳結果
    let json: serde_json::Value = response.json()?;

    // JSONから翻訳テキストを抽出
    // serde_json::Value はJSONの任意の値を表す型
    // [0][0][0] でネストされた配列にアクセスする
    let mut result = String::new();

    // レスポンスの最初の配列を取得
    if let Some(sentences) = json.get(0).and_then(|v| v.as_array()) {
        // 各文（センテンス）の翻訳を結合
        for sentence in sentences {
            if let Some(translated) = sentence.get(0).and_then(|v| v.as_str()) {
                result.push_str(translated);
            }
        }
    }

    if result.is_empty() {
        Err("翻訳結果を取得できませんでした".into())
    } else {
        Ok(result)
    }
}

/// テキストを翻訳する（メイン関数）
///
/// 設定に基づいて翻訳エンジンを選択し、言語を自動判定して翻訳する。
///
/// # 引数
/// - `text`: 翻訳するテキスト
/// - `config`: アプリケーション設定
///
/// # 戻り値
/// - `Ok(TranslationResult)`: 翻訳結果
/// - `Err(...)`: 翻訳エラー
pub fn translate(text: &str, config: &Config) -> Result<TranslationResult, Box<dyn std::error::Error>> {
    // 空白のみのテキストは翻訳しない
    let text = text.trim();
    if text.is_empty() {
        return Ok(TranslationResult {
            translated: String::new(),
            target_lang: String::new(),
        });
    }

    // 言語を自動判定
    let target = detect_target_lang(text, config);
    let source = &config.source_lang;

    // エンジンに応じて翻訳
    let translated = match config.engine.as_str() {
        "deepl" => {
            // DeepL はフェーズ3で実装予定
            // 今はエラーメッセージを返す
            if config.deepl_api_key.is_empty() {
                return Err("DeepL APIキーが未設定です。~/.quick-translate/config.json を編集してください".into());
            }
            // TODO: DeepL API 実装
            return Err("DeepL APIは未実装です。Google翻訳を使用してください".into());
        }
        _ => {
            // デフォルト: Google翻訳
            google_translate(text, &target, source)?
        }
    };

    Ok(TranslationResult {
        translated,
        target_lang: target,
    })
}
