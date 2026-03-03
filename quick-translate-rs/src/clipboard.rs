// clipboard.rs - クリップボード操作 & キー入力シミュレーション
//
// 選択テキスト翻訳のフロー:
// 1. ホットキーの修飾キー（Alt等）が物理的に離されるのを待つ
// 2. keybd_event で Ctrl+C をシミュレーション（選択テキストをコピー）
// 3. 少し待つ（クリップボードが更新されるのを待つ）
// 4. arboard でクリップボードからテキストを読む

use std::thread;
use std::time::Duration;

// Win32 API
#[cfg(windows)]
extern "system" {
    fn keybd_event(bVk: u8, bScan: u8, dwFlags: u32, dwExtraInfo: usize);
    fn GetAsyncKeyState(vKey: i32) -> i16;
}

/// キーイベントのフラグ
const KEYEVENTF_KEYUP: u32 = 0x0002;

/// 仮想キーコード
const VK_CONTROL: u8 = 0x11;
const VK_C: u8 = 0x43;
const VK_INSERT: u8 = 0x2D;
const VK_MENU: u8 = 0x12;    // Alt (generic)
const VK_LMENU: u8 = 0xA4;   // 左 Alt
const VK_RMENU: u8 = 0xA5;   // 右 Alt

/// 指定キーが物理的に押されているかチェック
/// GetAsyncKeyState の最上位ビット（0x8000）が立っていたら押下中
#[cfg(windows)]
fn is_key_pressed(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) & (0x8000u16 as i16) != 0 }
}

/// 全ての修飾キー（Alt）が物理的に離されるまで待つ
/// 最大1秒でタイムアウト
#[cfg(windows)]
fn wait_for_modifiers_release() {
    let start = std::time::Instant::now();
    loop {
        let alt_pressed = is_key_pressed(VK_MENU as i32)
            || is_key_pressed(VK_LMENU as i32)
            || is_key_pressed(VK_RMENU as i32);

        if !alt_pressed {
            break;
        }

        // タイムアウト（1秒）
        if start.elapsed().as_millis() > 1000 {
            eprintln!("[clipboard] Alt キーの解放待ちがタイムアウトしました");
            break;
        }

        thread::sleep(Duration::from_millis(10));
    }
    // 解放後さらに少し待って安定させる
    thread::sleep(Duration::from_millis(50));
}

/// Ctrl+C をシミュレートして選択テキストをクリップボードにコピーする
///
/// 1. まずホットキーの修飾キー（Alt）が物理的に離されるのを待つ
/// 2. Ctrl+C をシミュレート
///
/// 注意: UIPI（User Interface Privilege Isolation）により、
/// 管理者権限で動作しているウィンドウへの入力は失敗する場合がある。
#[cfg(windows)]
pub fn simulate_copy() {
    // まず物理的にキーが離されるのを待つ
    wait_for_modifiers_release();

    unsafe {
        // Ctrl+C をシミュレート
        keybd_event(VK_CONTROL, 0, 0, 0);
        keybd_event(VK_C, 0, 0, 0);
        keybd_event(VK_C, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// Windows 以外の環境用のスタブ
#[cfg(not(windows))]
pub fn simulate_copy() {
    eprintln!("simulate_copy は Windows でのみ動作します");
}

/// 選択テキストをコピーしてクリップボードから読み取る
///
/// 1. 既存クリップボードを保存
/// 2. クリップボードをクリア（コピー成功/失敗を判別するため）
/// 3. Ctrl+C をシミュレート
/// 4. クリップボードからテキストを読む
/// 5. 元のクリップボードを復元
///
/// # 戻り値
/// - `Some(String)`: コピーされたテキスト
/// - `None`: テキストの取得に失敗
pub fn copy_selected_text() -> Option<String> {
    // まず現在のクリップボードの内容を保存（後で復元するため）
    let old_clipboard = arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok());

    // クリップボードをマーカー文字列で上書き（コピー前の内容と区別するため）
    let marker = format!(
        "__QT_MARKER_{}_{}__",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(&marker);
    }
    thread::sleep(Duration::from_millis(30));

    // Ctrl+C は環境によって反映が遅れることがあるため、複数回リトライする
    for _ in 0..3 {
        // 1) Ctrl+C を試す
        simulate_copy();

        // 2) 失敗時フォールバックとして Ctrl+Insert を試す
        unsafe {
            keybd_event(VK_CONTROL, 0, 0, 0);
            keybd_event(VK_INSERT, 0, 0, 0);
            keybd_event(VK_INSERT, 0, KEYEVENTF_KEYUP, 0);
            keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, 0);
        }

        // 最大1200msポーリングして、マーカー以外のテキストが入るのを待つ
        let start = std::time::Instant::now();
        while start.elapsed().as_millis() < 1200 {
            let new_text = arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok());

            if let Some(text) = new_text {
                let trimmed = text.trim();
                if !trimmed.is_empty() && trimmed != marker {
                    // クリップボードを元に戻す
                    if let Some(old) = old_clipboard {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_text(&old);
                        }
                    }
                    return Some(trimmed.to_string());
                }
            }

            thread::sleep(Duration::from_millis(30));
        }

        thread::sleep(Duration::from_millis(50));
    }

    // コピー失敗時もクリップボードを復元
    if let Some(old) = old_clipboard {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(&old);
        }
    }

    None
}
