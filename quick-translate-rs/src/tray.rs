// tray.rs - システムトレイ常駐 & グローバルホットキー
//
// アーキテクチャ:
// - メインプロセス: トレイアイコン + ホットキーリスナー（Windowsメッセージループ）
// - ポップアップ: 別プロセスとして起動（eframeがメインスレッドを占有するため）
//
// ホットキー:
//   Ctrl+Shift+T → 入力ポップアップ（自分で文字を打つ）
//   Alt+Z → 選択テキスト翻訳（Ctrl+Cしてから翻訳→結果表示）

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIconBuilder, TrayIconEvent, Icon,
};

use std::env;
use std::fs;
use std::process::Command;

use crate::clipboard;

#[cfg(windows)]
struct TrayInstanceGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for TrayInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn acquire_tray_instance_lock(name: &str) -> Option<TrayInstanceGuard> {
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

        Some(TrayInstanceGuard { handle })
    }
}

#[cfg(not(windows))]
fn acquire_tray_instance_lock(_name: &str) -> Option<()> {
    Some(())
}

/// トレイアイコン用の16x16 "T" アイコンをRGBAデータから作成する
///
/// 外部アイコンファイルに依存せず、プログラム内で動的に生成する。
fn create_icon() -> Icon {
    let size = 16;
    // RGBA = 4バイト/ピクセル、初期値は全て0（透明な黒）
    let mut rgba = vec![0u8; size * size * 4];

    // "T" の文字を描画（水色: #6495ED = RGB(100, 149, 237)）
    let color = [100u8, 149, 237, 255]; // RGBA

    // 上のバー（y=2..5, x=2..14）
    for y in 2..5 {
        for x in 2..14 {
            let i = (y * size + x) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }
    // 縦棒（y=5..14, x=6..10）
    for y in 5..14 {
        for x in 6..10 {
            let i = (y * size + x) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }

    Icon::from_rgba(rgba, size as u32, size as u32)
        .expect("アイコンの作成に失敗")
}

/// 選択テキスト翻訳を実行する
///
/// 1. Ctrl+C シミュレーション → クリップボードから読み取り
/// 2. リクエストファイルにテキストを書き出し
/// 3. 別プロセスでポップアップを起動（既に開いていれば既存ポップアップが内容更新）
fn handle_selected_translation() {
    // 選択テキストをコピー&取得
    let text = match clipboard::copy_selected_text() {
        Some(t) => t,
        None => {
            eprintln!("選択テキストの取得に失敗");
            return;
        }
    };

    if text.is_empty() {
        return;
    }

    // 既存ポップアップが監視する固定ファイルに書き込む
    let request_path = env::temp_dir().join("quick_translate_popup_request.txt");

    if let Err(e) = fs::write(&request_path, text) {
        eprintln!("リクエストファイルの作成に失敗: {}", e);
        return;
    }

    // 既存ポップアップが無ければ新規起動。既にある場合は単一起動ロックで即終了。
    spawn_self(&["--popup"]);
}

/// 自分自身を別プロセスとして起動する
///
/// eframe::run_native() はメインスレッドをブロックするため、
/// ポップアップは別プロセスで起動する必要がある。
fn spawn_self(args: &[&str]) {
    match env::current_exe() {
        Ok(exe) => {
            let _ = Command::new(exe).args(args).spawn();
        }
        Err(e) => eprintln!("自分自身のパス取得に失敗: {}", e),
    }
}

/// システムトレイアプリのメインループを実行する
///
/// この関数は終了するまでブロックする（アプリのメインループ）。
pub fn run_tray() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = match acquire_tray_instance_lock("QuickTranslateTraySingleton") {
        Some(g) => g,
        None => {
            println!("Quick Translate は既に起動しています");
            return Ok(());
        }
    };

    // --- メニューの作成 ---
    let menu = Menu::new();
    let item_popup = MenuItem::new("ポップアップを開く (Ctrl+Shift+T)", true, None);
    let item_quit = MenuItem::new("終了", true, None);
    menu.append(&item_popup)?;
    menu.append(&item_quit)?;

    let item_popup_id = item_popup.id().clone();
    let item_quit_id = item_quit.id().clone();

    // --- トレイアイコンの作成 ---
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Quick Translate")
        .with_icon(create_icon())
        .build()?;

    // --- グローバルホットキーの登録 ---
    let hotkey_manager = GlobalHotKeyManager::new()?;

    // Ctrl+Shift+T → 入力ポップアップ
    let hk_popup = HotKey::new(
        Some(Modifiers::CONTROL | Modifiers::SHIFT),
        Code::KeyT,
    );
    // Alt+Z → 選択テキスト翻訳（右Altでも左Altでも発火する）
    let hk_selected = HotKey::new(
        Some(Modifiers::ALT),
        Code::KeyZ,
    );

    hotkey_manager.register(hk_popup)?;
    hotkey_manager.register(hk_selected)?;

    println!("Quick Translate がシステムトレイで起動しました");
    println!("  Ctrl+Shift+T: ポップアップを開く");
    println!("  Alt+Z:        選択テキストを翻訳");

    // ホットキー連打防止用のタイムスタンプ
    // 短時間のチャタリングのみ抑止し、通常の連続操作は通す
    let mut last_hotkey_time = std::time::Instant::now() - std::time::Duration::from_secs(10);

    // --- Windows メッセージループ ---
    // global-hotkey は RegisterHotKey を使うため、
    // WM_HOTKEY メッセージを処理するメッセージループが必須。
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG,
        };

        loop {
            // GetMessageW: メッセージキューからメッセージを取得（ブロッキング）
            // 戻り値: >0=メッセージあり, 0=WM_QUIT, <0=エラー
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if ret <= 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // --- ホットキーイベントの処理 ---
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                // 連打防止: 前回から300ms以内のイベントは無視
                if last_hotkey_time.elapsed().as_millis() < 300 {
                    continue;
                }
                last_hotkey_time = std::time::Instant::now();

                if event.id == hk_popup.id() {
                    // Ctrl+Shift+T: 入力ポップアップを別プロセスで起動
                    spawn_self(&["--popup"]);
                } else if event.id == hk_selected.id() {
                    // Ctrl+Q: 選択テキスト翻訳
                    handle_selected_translation();
                }
            }

            // --- トレイメニューイベントの処理 ---
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == item_popup_id {
                    spawn_self(&["--popup"]);
                } else if event.id == item_quit_id {
                    // 終了
                    return Ok(());
                }
            }

            // トレイアイコンイベントを消費する（溜まるとメモリリークするため）
            while TrayIconEvent::receiver().try_recv().is_ok() {}
        }
    }

    // Windows以外（コンパイルは通るが実際には動かない）
    #[cfg(not(windows))]
    {
        eprintln!("トレイモードは Windows でのみ動作します");
    }

    Ok(())
}
