// popup.rs - egui ポップアップ翻訳ウィンドウ
//
// Alfred/Spotlight風のボーダーレスポップアップウィンドウ。
// テキスト入力 → リアルタイム翻訳 → 結果表示。
// Python版の TranslatePopup クラスに相当する。

use eframe::egui;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::translator;

// ------------------------------------------------------------
// 単一起動ロック（ポップアップ多重起動防止）
// ------------------------------------------------------------

#[cfg(windows)]
struct PopupInstanceGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for PopupInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn acquire_popup_instance_lock(name: &str) -> Option<PopupInstanceGuard> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let mut name_wide: Vec<u16> = name.encode_utf16().collect();
    name_wide.push(0);

    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 1, name_wide.as_ptr());
        if handle.is_null() {
            return None;
        }

        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = windows_sys::Win32::Foundation::CloseHandle(handle);
            return None;
        }

        Some(PopupInstanceGuard { handle })
    }
}

#[cfg(not(windows))]
fn acquire_popup_instance_lock(_name: &str) -> Option<()> {
    Some(())
}

fn popup_request_file_path() -> std::path::PathBuf {
    std::env::temp_dir().join("quick_translate_popup_request.txt")
}

fn consume_popup_request(path: &std::path::Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let _ = fs::remove_file(path);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(windows)]
fn is_current_process_foreground() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, &mut pid);
        pid == GetCurrentProcessId()
    }
}

#[cfg(not(windows))]
fn is_current_process_foreground() -> bool {
    true
}

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

    /// 一度でもフォーカスを得たか（フォーカス喪失時クローズ判定用）
    had_focus: bool,

    /// Alt+Z のリクエストファイル（既存ポップアップ更新用）
    request_file_path: std::path::PathBuf,

    /// 外部リクエストファイルの前回ポーリング時刻
    last_request_poll: Instant,

    /// ポップアップ生成時刻（フォーカス未取得時のタイムアウト判定用）
    created_at: Instant,

    /// 最後に適用したウィンドウサイズ（過剰なリサイズを防ぐ）
    last_viewport_size: Option<(f32, f32)>,
}

impl TranslatePopup {
    /// 新しいポップアップウィンドウを作成する
    pub fn new(config: Config, initial_text: String) -> Self {
        let request_file_path = popup_request_file_path();
        let initial_text = if initial_text.trim().is_empty() {
            consume_popup_request(&request_file_path).unwrap_or_default()
        } else {
            initial_text
        };

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
            had_focus: false,
            request_file_path,
            last_request_poll: Instant::now() - Duration::from_secs(1),
            created_at: Instant::now(),
            last_viewport_size: None,
        }
    }

    /// Alt+Z の新規リクエストを読み取り、表示中の内容を更新する
    fn check_external_request(&mut self) {
        if self.last_request_poll.elapsed() < Duration::from_millis(80) {
            return;
        }
        self.last_request_poll = Instant::now();

        let Some(text) = consume_popup_request(&self.request_file_path) else {
            return;
        };

        if text == self.input_text {
            return;
        }

        self.input_text = text;
        self.first_frame = true;
        self.start_translation();
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

    fn adjust_viewport_size(&mut self, ctx: &egui::Context) {
        let (width, height) =
            estimate_live_popup_size(&self.input_text, &self.result_text, self.config.font_size);
        let should_resize = match self.last_viewport_size {
            None => true,
            Some((w, h)) => (w - width).abs() > 6.0 || (h - height).abs() > 6.0,
        };

        if should_resize {
            self.last_viewport_size = Some((width, height));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
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
        // 一度フォーカスを得た後に他アプリへフォーカスが移ったら自動で閉じる
        let focused = is_current_process_foreground();
        if focused {
            self.had_focus = true;
        } else if self.had_focus || self.created_at.elapsed() > Duration::from_millis(1500) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Alt+Z の新規リクエストを反映（既存ウィンドウを再利用）
        self.check_external_request();

        // 翻訳結果のチェック（毎フレーム）
        self.check_translation_result();

        // テキスト量に応じてポップアップサイズを調整
        self.adjust_viewport_size(ctx);

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
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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

fn count_wrapped_lines(text: &str, chars_per_line: usize) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    let per_line = chars_per_line.max(8);
    let mut lines = 0usize;
    for line in text.lines() {
        let n = line.chars().count().max(1);
        lines += ((n + per_line - 1) / per_line).max(1);
    }
    lines.max(1)
}

fn estimate_live_popup_size(input_text: &str, result_text: &str, font_size: f32) -> (f32, f32) {
    let char_width = font_size * 0.68;
    let width_chars = if !result_text.trim().is_empty() {
        // 結果表示中は「結果本文」を優先して幅を決める（横長化を防ぐ）
        let result_max = result_text
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(24);
        let has_spaces = result_text.contains(' ');
        let preferred = if has_spaces { 52 } else { 34 };
        result_max.min(preferred + 14).max(28)
    } else {
        // 結果がない間は入力の長さを見るが、過剰に広げない
        let input_max = input_text
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(24)
            .min(60);
        input_max.max(24)
    };
    let width = (width_chars as f32 * char_width + 88.0).clamp(520.0, 860.0);

    let text_area_width = (width - 32.0).max(220.0);
    let chars_per_line = (text_area_width / char_width).floor() as usize;
    let result_lines = count_wrapped_lines(result_text, chars_per_line);
    let result_height = result_lines as f32 * font_size * 1.45;

    let height = (32.0  // top bar
        + 8.0          // spacing
        + 32.0         // input
        + 12.0         // spacing
        + result_height
        + 28.0)        // bottom padding
        .clamp(190.0, 840.0);

    (width, height)
}

/// ポップアップウィンドウを表示する
///
/// eframe::run_native() でネイティブウィンドウを起動する。
/// この関数はウィンドウが閉じるまでブロックする。
pub fn show_popup(config: Config, initial_text: String) -> Result<(), Box<dyn std::error::Error>> {
    // 既にポップアップが開いている場合は新規起動しない
    let _guard = match acquire_popup_instance_lock("QuickTranslatePopupSingleton") {
        Some(g) => g,
        None => return Ok(()),
    };

    let (initial_width, initial_height) =
        estimate_live_popup_size(&initial_text, "", config.font_size);

    // ウィンドウのオプション設定
    let options = eframe::NativeOptions {
        // ウィンドウサイズ
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([initial_width, initial_height])
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

// ============================================================
// 結果表示ポップアップ（選択テキスト翻訳用）
// ============================================================

/// 結果表示専用ポップアップの状態
///
/// 入力フィールドなし。翻訳結果と原文を表示するだけ。
/// Esc で閉じる、Ctrl+Enter で翻訳結果をコピー。
struct ResultPopup {
    /// 翻訳結果テキスト
    translated: String,
    /// 原文テキスト
    original: String,
    /// フォントサイズ
    font_size: f32,

    /// 一度でもフォーカスを得たか（フォーカス喪失時クローズ判定用）
    had_focus: bool,

    /// ポップアップ生成時刻（フォーカス未取得時のタイムアウト判定用）
    created_at: Instant,
}

impl eframe::App for ResultPopup {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 一度フォーカスを得た後に他アプリへフォーカスが移ったら自動で閉じる
        let focused = is_current_process_foreground();
        if focused {
            self.had_focus = true;
        } else if self.had_focus || self.created_at.elapsed() > Duration::from_millis(1500) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Esc で閉じる
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Ctrl+Enter でコピーして閉じる
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)) {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(&self.translated);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // --- 色定義 ---
        let bg_color = egui::Color32::from_rgb(30, 30, 46);       // #1e1e2e
        let result_color = egui::Color32::from_rgb(166, 227, 161); // #a6e3a1
        let original_color = egui::Color32::from_rgb(108, 112, 134); // #6c7086
        let hint_color = egui::Color32::from_rgb(88, 91, 112);     // #585b70

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(bg_color)
                    .inner_margin(egui::Margin::same(16))
            )
            .show(ctx, |ui| {
                // --- 翻訳結果（メイン表示、大きく緑で） ---
                ui.colored_label(
                    result_color,
                    egui::RichText::new(&self.translated)
                        .size(self.font_size),
                );

                ui.add_space(8.0);

                // --- 区切り線 ---
                ui.separator();

                ui.add_space(4.0);

                // --- 原文（小さくグレーで） ---
                ui.colored_label(
                    original_color,
                    egui::RichText::new(&self.original)
                        .size(self.font_size * 0.75),
                );

                ui.add_space(8.0);

                // --- ヒント ---
                ui.colored_label(
                    hint_color,
                    egui::RichText::new("Ctrl+Enter=コピー | Esc=閉じる")
                        .size(10.0),
                );
            });
    }
}

/// テキストの内容からウィンドウサイズを推定する
///
/// 文字数と行数に基づいて、全文が読めるサイズを計算する。
fn estimate_window_size(text: &str, original: &str, font_size: f32) -> (f32, f32) {
    // 両方のテキストで最長の行を探す
    let all_text = format!("{}\n{}", text, original);
    let lines: Vec<&str> = all_text.lines().collect();
    let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(10);

    // 文字幅の推定（日本語は全角なので font_size に近い幅、英語は約半分）
    // 平均的に 0.7 * font_size を1文字幅とする
    let char_width = font_size * 0.7;
    let width = (max_chars as f32 * char_width + 64.0) // 64 = 左右パディング
        .clamp(350.0, 900.0);

    // 高さ: 翻訳結果の行数 + 原文の行数 + ヘッダー/フッター
    let translated_lines = text.lines().count().max(1);
    let original_lines = original.lines().count().max(1);
    let line_height = font_size * 1.5;
    let original_line_height = font_size * 0.75 * 1.5;
    let height = (translated_lines as f32 * line_height
        + original_lines as f32 * original_line_height
        + 80.0) // パディング + 区切り線 + ヒント
        .clamp(120.0, 600.0);

    (width, height)
}

/// 結果表示ポップアップを表示する
///
/// テンプファイルから翻訳結果と原文を読み込んで表示する。
/// ファイル形式:
///   1行目以降〜"---"まで: 翻訳結果
///   "---"以降: 原文
///
/// # 引数
/// - `result_file`: テンプファイルのパス
/// - `config`: アプリケーション設定
pub fn show_result_popup(result_file: &str, config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // 既にポップアップが開いている場合は新規起動しない
    let _guard = match acquire_popup_instance_lock("QuickTranslatePopupSingleton") {
        Some(g) => g,
        None => return Ok(()),
    };

    // テンプファイルを読む
    let content = fs::read_to_string(result_file)?;
    // 読み終わったら削除
    let _ = fs::remove_file(result_file);

    // "---" で分割
    let mut translated = String::new();
    let mut original = String::new();
    let mut is_original = false;

    for line in content.lines() {
        if line.trim() == "---" {
            is_original = true;
            continue;
        }
        if is_original {
            if !original.is_empty() {
                original.push('\n');
            }
            original.push_str(line);
        } else {
            if !translated.is_empty() {
                translated.push('\n');
            }
            translated.push_str(line);
        }
    }

    // ウィンドウサイズを推定
    let (width, height) = estimate_window_size(&translated, &original, config.font_size);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width, height])
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false),
        ..Default::default()
    };

    let font_size = config.font_size;

    eframe::run_native(
        "Quick Translate Result",
        options,
        Box::new(move |cc| {
            setup_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(ResultPopup {
                translated,
                original,
                font_size,
                had_focus: false,
                created_at: Instant::now(),
            }) as Box<dyn eframe::App>)
        }),
    )?;

    Ok(())
}
