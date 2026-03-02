// main.rs - Quick Translate エントリポイント
//
// CLIモード:
//   quick-translate --translate "Hello"   → 翻訳して標準出力
//   quick-translate --popup               → ポップアップウィンドウを表示
//   quick-translate --popup-file <path>   → ファイルからテキストを読んでポップアップ表示
//   quick-translate                       → ポップアップウィンドウを表示
//
// フェーズ2でシステムトレイ + ホットキーを追加予定。

// モジュール宣言
// `mod xxx;` で src/xxx.rs を読み込む
mod config;
mod lang;
mod popup;
mod translator;

use std::env;
use std::fs;

/// コマンドライン引数をパースする簡易パーサー
///
/// 本格的なCLIパーサー（clap等）は使わず、
/// std::env::args() で手動パースする。
/// Rust学習の一環として、イテレータの使い方を学べる。
struct CliArgs {
    /// --translate "text": 翻訳するテキスト
    translate_text: Option<String>,

    /// --popup: ポップアップ表示
    show_popup: bool,

    /// --popup-file <path>: ファイルからテキストを読んでポップアップ表示
    popup_file: Option<String>,

    /// --engine <name>: エンジン指定（google/deepl）
    engine: Option<String>,
}

impl CliArgs {
    /// コマンドライン引数をパースする
    fn parse() -> Self {
        // env::args() はプログラム名を含む引数のイテレータ
        // .skip(1) でプログラム名をスキップ
        // .collect() で Vec<String> に変換
        let args: Vec<String> = env::args().skip(1).collect();

        let mut cli = CliArgs {
            translate_text: None,
            show_popup: false,
            popup_file: None,
            engine: None,
        };

        // イテレータを使って引数を順番に処理
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--translate" | "-t" => {
                    // 次の引数がテキスト
                    if i + 1 < args.len() {
                        cli.translate_text = Some(args[i + 1].clone());
                        i += 1; // 次の引数をスキップ
                    }
                }
                "--popup" | "-p" => {
                    cli.show_popup = true;
                }
                "--popup-file" => {
                    if i + 1 < args.len() {
                        cli.popup_file = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--engine" | "-e" => {
                    if i + 1 < args.len() {
                        cli.engine = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--help" | "-h" => {
                    println!("Quick Translate - Windows翻訳ツール (Rust版)");
                    println!();
                    println!("使い方:");
                    println!("  quick-translate                       ポップアップを表示");
                    println!("  quick-translate --translate \"text\"     テキストを翻訳");
                    println!("  quick-translate --popup               ポップアップを表示");
                    println!("  quick-translate --popup-file <path>   ファイルからテキストを読んで翻訳");
                    println!("  quick-translate --engine google|deepl エンジンを指定");
                    std::process::exit(0);
                }
                _ => {
                    // 不明な引数は無視
                    eprintln!("不明な引数: {}", args[i]);
                }
            }
            i += 1;
        }

        cli
    }
}

/// メイン関数
///
/// Rustプログラムのエントリポイント。
/// 戻り値の `Result<(), Box<dyn std::error::Error>>` は:
/// - `()`: 成功時は何も返さない
/// - `Box<dyn std::error::Error>`: 任意のエラー型
/// これにより、`?` 演算子でエラーを簡潔に処理できる。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 引数をパース
    let args = CliArgs::parse();

    // 設定を読み込む
    let mut config = config::load_config();

    // エンジン指定があれば上書き
    if let Some(engine) = args.engine {
        config.engine = engine;
    }

    // モードに応じた処理の分岐
    if let Some(text) = args.translate_text {
        // --translate モード: テキストを翻訳して標準出力
        let result = translator::translate(&text, &config)?;
        println!("{}", result.translated);

        // クリップボードにもコピー
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(&result.translated);
        }
    } else if let Some(file_path) = args.popup_file {
        // --popup-file モード: ファイルからテキストを読む
        let initial_text = match fs::read_to_string(&file_path) {
            Ok(text) => {
                // ファイルを読んだ後に削除（Python版と同じ動作）
                let _ = fs::remove_file(&file_path);
                text.trim().to_string()
            }
            Err(_) => String::new(),
        };
        popup::show_popup(config, initial_text)?;
    } else {
        // デフォルト or --popup: ポップアップ表示
        popup::show_popup(config, String::new())?;
    }

    Ok(())
}
