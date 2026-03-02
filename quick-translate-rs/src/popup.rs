// popup.rs - egui ポップアップ翻訳ウィンドウ
//
// Alfred/Spotlight風のボーダーレスポップアップウィンドウ。
// テキスト入力 → リアルタイム翻訳 → 結果表示。
// Python版の TranslatePopup クラスに相当する。

use eframe::egui;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::config::Config;
use crate::translator;

/// Windows のシステムフォントから日本語対応フォントを読み込む
///
/// egui のデフォルトフォントはラテン文字のみ対応。
/// CJK（日中韓）文字を表示するには、追加フォントの登録が必要。
/// Windows の Fonts フォルダから順番に探して、最初に見つかったものを使う。
fn setup_japanese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Windows でよく使われる日本語フォントの候補（優先順）
    let font_candidates = [
        r"C:\Windows\Fonts\meiryo.ttc",     // メイリオ（見やすい）
        r"C:\Windows\Fonts\YuGothM.ttc",    // 游ゴシック Medium
        r"C:\Windows\Fonts\msgothic.ttc",    // MS ゴシック
        r"C:\Windows\Fonts\msmincho.ttc",    // MS 明朝
    ];

    for path in &font_candidates {
        if let Ok(font_data) = fs::read(path) {
            // フォントデータを登録
            // "jp_font" という名前で登録し、Proportional（可変幅）フォントファミリーに追加
            fonts.font_data.insert(
                "jp_font".to_owned(),
                egui::FontData::from_owned(font_data).into(),
            );

            // Proportional フォントファミリーに日本語フォントを追加
            // 既存の英語フォントの「後に」追加することで、
            // 英語 → デフォルトフォント、日本語 → jp_font とフォールバックする
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.push("jp_font".to_owned());
            }

            // Monospace フォントファミリーにも追加（入力フィールド用）
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("jp_font".to_owned());
            }

            ctx.set_fonts(fonts);
            return;
        }
    }

    // どのフォントも見つからなかった場合（通常ありえない）
    eprintln!("警告: 日本語フォントが見つかりませんでした");
}

/// ポップアップウィンドウの状態を管理する構造体
///
/// eframe::App トレイトを実装することで、egui のアプリとして動作する。
pub struct TranslatePopup {
    /// アプリケーション設定
    config: Config,

    /// 入力テキスト
    input_text: String,

    /// 翻訳結果テキスト
    result_text: String,

    /// 現在使用中のエンジン名
    current_engine: String,

    /// 最後に入力が変更された時刻（デバウンス用）
    last_input_change: Option<Instant>,

    /// 翻訳が進行中かどうか
    is_translating: bool,

    /// バックグラウンド翻訳スレッドからの結果受信チャネル
    /// mpsc = Multi-Producer, Single-Consumer（複数送信、単一受信）
    result_receiver: Option<mpsc::Receiver<Result<translator::TranslationResult, String>>>,

    /// 初回フォーカス用フラグ
    first_frame: bool,

    /// クリップボードにコピーするリクエスト
    copy_requested: bool,
}

impl TranslatePopup {
    /// 新しいポップアップウィンドウを作成する
    pub fn new(config: Config, initial_text: String) -> Self {
        Self {
            current_engine: config.engine.clone(),
            config,
            input_text: initial_text.clone(),
            result_text: String::new(),
            last_input_change: if initial_text.is_empty() {
                None
            } else {
                // 初期テキストがあれば即座に翻訳を開始
                Some(Instant::now() - std::time::Duration::from_secs(1))
            },
            is_translating: false,
            result_receiver: None,
            first_frame: true,
            copy_requested: false,
        }
    }

    /// バックグラウンドスレッドで翻訳を実行する
    ///
    /// メインスレッド（UIスレッド）をブロックしないために、
    /// 翻訳処理は別スレッドで実行し、結果をチャネル経由で受け取る。
    fn start_translation(&mut self) {
        let text = self.input_text.trim().to_string();
        if text.is_empty() {
            self.result_text.clear();
            return;
        }

        self.is_translating = true;
        self.result_text = "翻訳中...".to_string();

        // チャネルを作成（送信側 tx, 受信側 rx）
        let (tx, rx) = mpsc::channel();
        self.result_receiver = Some(rx);

        // 設定をクローンしてスレッドに渡す
        // Rustでは別スレッドにデータを渡すとき、所有権の移動が必要
        // clone() でコピーを作ることで、元のデータも引き続き使える
        let config = self.config.clone();

        // 新しいスレッドを生成
        thread::spawn(move || {
            // move クロージャ: text と config の所有権をスレッドに移動
            let result = translator::translate(&text, &config);
            // 結果をチャネル経由で送信（エラーは無視）
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
    }

    /// チャネルから翻訳結果を受け取る（ノンブロッキング）
    fn check_translation_result(&mut self) {
        if let Some(ref rx) = self.result_receiver {
            // try_recv(): ブロックせずに受信を試みる
            // Ok(msg) → メッセージを受信
            // Err(TryRecvError::Empty) → まだ結果がない
            // Err(TryRecvError::Disconnected) → 送信側が切断された
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    self.result_text = result.translated;
                    self.is_translating = false;
                    self.result_receiver = None;
                }
                Ok(Err(error)) => {
                    self.result_text = format!("エラー: {}", error);
                    self.is_translating = false;
                    self.result_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // まだ翻訳中 → 何もしない
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.result_text = "翻訳スレッドが予期せず終了しました".to_string();
                    self.is_translating = false;
                    self.result_receiver = None;
                }
            }
        }
    }
}

/// eframe::App トレイトの実装
///
/// egui は「イミディエイトモード」GUI。
/// 毎フレーム update() が呼ばれ、その中でUIを構築する。
/// 「状態が変わったら再描画」ではなく「毎フレーム全部描き直す」方式。
impl eframe::App for TranslatePopup {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 翻訳結果のチェック（毎フレーム）
        self.check_translation_result();

        // デバウンス: 最後の入力変更から400ms後に翻訳を開始
        if let Some(last_change) = self.last_input_change {
            if last_change.elapsed().as_millis() >= 400 && !self.is_translating {
                self.last_input_change = None;
                self.start_translation();
            }
        }

        // キーボードショートカットの処理
        // Esc キーで閉じる
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Ctrl+Enter でクリップボードにコピー
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)) {
            if !self.result_text.is_empty() && self.result_text != "翻訳中..." {
                self.copy_requested = true;
            }
        }

        // クリップボードコピーの実行
        if self.copy_requested {
            self.copy_requested = false;
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(&self.result_text);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // --------------------------------
        // UI の構築
        // --------------------------------

        // 背景色の設定（Python版と同じダークテーマ）
        let bg_color = egui::Color32::from_rgb(30, 30, 46);       // #1e1e2e
        let fg_color = egui::Color32::from_rgb(205, 214, 244);    // #cdd6f4
        let accent_color = egui::Color32::from_rgb(137, 180, 250); // #89b4fa
        let result_color = egui::Color32::from_rgb(166, 227, 161); // #a6e3a1
        let hint_color = egui::Color32::from_rgb(108, 112, 134);   // #6c7086

        // CentralPanel: 画面全体を覆うパネル
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(bg_color)
                    .inner_margin(egui::Margin::same(16))
            )
            .show(ctx, |ui| {
                // --- 上部バー: エンジン名 + ヒント ---
                ui.horizontal(|ui| {
                    ui.colored_label(
                        accent_color,
                        egui::RichText::new(format!("⚡ {}", self.current_engine.to_uppercase()))
                            .size(12.0)
                            .strong(),
                    );

                    // 右寄せのヒントテキスト
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(
                            hint_color,
                            egui::RichText::new("Ctrl+Enter=コピー | Esc=閉じる").size(10.0),
                        );
                    });
                });

                ui.add_space(8.0);

                // --- 入力フィールド ---
                let input_response = ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::TextEdit::singleline(&mut self.input_text)
                        .font(egui::TextStyle::Heading)
                        .text_color(fg_color)
                        .hint_text(
                            egui::RichText::new("翻訳するテキストを入力...")
                                .color(hint_color),
                        ),
                );

                // 初回フレームで入力フィールドにフォーカス
                if self.first_frame {
                    input_response.request_focus();
                    self.first_frame = false;
                }

                // テキストが変更されたらデバウンスタイマーをリセット
                if input_response.changed() {
                    self.last_input_change = Some(Instant::now());
                }

                ui.add_space(12.0);

                // --- 翻訳結果 ---
                if !self.result_text.is_empty() {
                    let color = if self.result_text.starts_with("エラー") || self.result_text.starts_with("翻訳中") {
                        hint_color
                    } else {
                        result_color
                    };

                    ui.colored_label(
                        color,
                        egui::RichText::new(&self.result_text)
                            .size(self.config.font_size),
                    );
                }
            });

        // 翻訳中は連続して再描画をリクエスト（結果チェックのため）
        if self.is_translating || self.last_input_change.is_some() {
            ctx.request_repaint();
        }
    }
}

/// ポップアップウィンドウを表示する
///
/// eframe::run_native() でネイティブウィンドウを起動する。
/// この関数はウィンドウが閉じるまでブロックする。
pub fn show_popup(config: Config, initial_text: String) -> Result<(), Box<dyn std::error::Error>> {
    // ウィンドウのオプション設定
    let options = eframe::NativeOptions {
        // ウィンドウサイズ
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 200.0])
            .with_decorations(false)        // タイトルバーなし（ボーダーレス）
            .with_always_on_top()           // 常に最前面
            .with_transparent(true)         // 背景透過を有効化
            .with_resizable(false),         // リサイズ不可

        ..Default::default()
    };

    // eframe アプリケーションを起動
    // Box::new() でヒープにアロケートする（eframeの要件）
    eframe::run_native(
        "Quick Translate",
        options,
        Box::new(move |cc| {
            // 日本語フォントを読み込む（CJK文字の表示に必須）
            setup_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(TranslatePopup::new(config, initial_text)) as Box<dyn eframe::App>)
        }),
    )?;

    Ok(())
}
