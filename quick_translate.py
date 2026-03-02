"""
Quick Translate - Windows Translation Tool
Alfred Quick Translate inspired translation tool for Windows.
System tray resident app with popup translation window.

Usage:
  python quick_translate.py                  # Launch system tray app
  python quick_translate.py --translate "text"  # Translate text directly (for AHK)
  python quick_translate.py --popup          # Show popup window (for AHK)
  python quick_translate.py --selected       # Translate selected text (for AHK)
"""

import sys
import os
import json
import re
import threading
import time
import tkinter as tk
from tkinter import ttk, font as tkfont
from datetime import datetime
from pathlib import Path
import ctypes
import subprocess
import argparse

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------
CONFIG_DIR = Path.home() / ".quick-translate"
CONFIG_FILE = CONFIG_DIR / "config.json"
LOG_FILE = Path.home() / "translate_log.yml"

DEFAULT_CONFIG = {
    "engine": "google",          # "google" or "deepl"
    "deepl_api_key": "",
    "source_lang": "auto",       # auto-detect
    "target_lang_ja": "en",      # when source is Japanese
    "target_lang_en": "ja",      # when source is English
    "popup_width": 600,
    "popup_height": 160,
    "font_size": 13,
    "opacity": 0.95,
    "log_enabled": True,
    "hotkey_popup": "ctrl+shift+t",
    "hotkey_selected": "ctrl+shift+y",
}


def load_config() -> dict:
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    if CONFIG_FILE.exists():
        with open(CONFIG_FILE, "r", encoding="utf-8") as f:
            cfg = json.load(f)
        # merge with defaults for any missing keys
        for k, v in DEFAULT_CONFIG.items():
            cfg.setdefault(k, v)
        return cfg
    else:
        save_config(DEFAULT_CONFIG)
        return dict(DEFAULT_CONFIG)


def save_config(cfg: dict):
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    with open(CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(cfg, f, indent=2, ensure_ascii=False)


# ---------------------------------------------------------------------------
# Language Detection (simple heuristic, no external deps)
# ---------------------------------------------------------------------------
def is_japanese(text: str) -> bool:
    """Check if text contains Japanese characters."""
    for ch in text:
        if '\u3040' <= ch <= '\u309F':  # Hiragana
            return True
        if '\u30A0' <= ch <= '\u30FF':  # Katakana
            return True
        if '\u4E00' <= ch <= '\u9FFF':  # CJK Unified Ideographs
            return True
    return False


def detect_target_lang(text: str, config: dict) -> str:
    """Auto-detect: if Japanese → translate to English, else → translate to Japanese."""
    if is_japanese(text):
        return config.get("target_lang_ja", "en")
    return config.get("target_lang_en", "ja")


# ---------------------------------------------------------------------------
# Translation Engines
# ---------------------------------------------------------------------------
class GoogleTranslator:
    """Google Translate via deep-translator library."""

    def translate(self, text: str, target: str, source: str = "auto") -> str:
        from deep_translator import GoogleTranslator as GT
        result = GT(source=source, target=target).translate(text)
        return result or ""


class DeepLTranslator:
    """DeepL API translator."""

    def __init__(self, api_key: str):
        self.api_key = api_key

    def translate(self, text: str, target: str, source: str = "auto") -> str:
        import deepl
        translator = deepl.Translator(self.api_key)
        # DeepL uses uppercase lang codes like "EN", "JA"
        target_upper = target.upper()
        # DeepL requires "EN-US" or "EN-GB" for English target
        if target_upper == "EN":
            target_upper = "EN-US"
        source_lang = None if source == "auto" else source.upper()
        result = translator.translate_text(text, target_lang=target_upper, source_lang=source_lang)
        return str(result)


def get_translator(config: dict):
    engine = config.get("engine", "google")
    if engine == "deepl":
        api_key = config.get("deepl_api_key", "")
        if not api_key:
            raise ValueError("DeepL API key is not set. Edit ~/.quick-translate/config.json")
        return DeepLTranslator(api_key)
    return GoogleTranslator()


def do_translate(text: str, config: dict) -> tuple[str, str]:
    """Translate text. Returns (translated_text, target_lang)."""
    text = text.strip()
    if not text:
        return "", ""
    target = detect_target_lang(text, config)
    translator = get_translator(config)
    result = translator.translate(text, target=target)
    return result, target


# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------
def log_translation(original: str, translated: str, engine: str, target_lang: str, config: dict):
    if not config.get("log_enabled", True):
        return
    entry = (
        f"- timestamp: \"{datetime.now().isoformat()}\"\n"
        f"  engine: \"{engine}\"\n"
        f"  target: \"{target_lang}\"\n"
        f"  original: \"{original.replace(chr(10), ' ')}\"\n"
        f"  translated: \"{translated.replace(chr(10), ' ')}\"\n"
    )
    try:
        with open(LOG_FILE, "a", encoding="utf-8") as f:
            f.write(entry + "\n")
    except Exception as e:
        print(f"Log error: {e}", file=sys.stderr)


# ---------------------------------------------------------------------------
# Clipboard Helpers
# ---------------------------------------------------------------------------
def get_clipboard() -> str:
    """Get clipboard text using tkinter."""
    root = tk.Tk()
    root.withdraw()
    try:
        text = root.clipboard_get()
    except tk.TclError:
        text = ""
    root.destroy()
    return text


def set_clipboard(text: str):
    """Set clipboard text using tkinter."""
    root = tk.Tk()
    root.withdraw()
    root.clipboard_clear()
    root.clipboard_append(text)
    root.update()
    root.destroy()


# ---------------------------------------------------------------------------
# Popup Translation Window
# ---------------------------------------------------------------------------
class TranslatePopup:
    """Spotlight/Alfred-style translation popup."""

    def __init__(self, config: dict, initial_text: str = "", on_close=None):
        self.config = config
        self.on_close = on_close
        self.translate_timer = None
        self.current_engine = config.get("engine", "google")

        self.root = tk.Tk()
        self.root.title("Quick Translate")
        self.root.overrideredirect(True)  # borderless
        self.root.attributes("-topmost", True)
        self.root.attributes("-alpha", config.get("opacity", 0.95))

        # DPI awareness
        try:
            ctypes.windll.shcore.SetProcessDpiAwareness(1)
        except Exception:
            pass

        # Sizing
        self.min_w = 500
        self.max_w = 1000
        self.min_h = 130
        self.max_h = 500
        self.fsize = config.get("font_size", 13)
        w = self.min_w
        h = self.min_h
        sw = self.root.winfo_screenwidth()
        sh = self.root.winfo_screenheight()
        x = (sw - w) // 2
        y = (sh - h) // 3  # upper third
        self.root.geometry(f"{w}x{h}+{x}+{y}")
        self.screen_w = sw
        self.screen_h = sh

        # Colors
        bg = "#1e1e2e"
        fg = "#cdd6f4"
        accent = "#89b4fa"
        input_bg = "#313244"
        result_fg = "#a6e3a1"

        self.root.configure(bg=bg)

        # Rounded appearance via frame
        main_frame = tk.Frame(self.root, bg=bg, padx=16, pady=12)
        main_frame.pack(fill=tk.BOTH, expand=True)

        # Top bar: engine indicator + close hint
        top_frame = tk.Frame(main_frame, bg=bg)
        top_frame.pack(fill=tk.X, pady=(0, 6))

        self.engine_label = tk.Label(
            top_frame, text=f"⚡ {self.current_engine.upper()}",
            bg=bg, fg=accent,
            font=("Segoe UI", 9, "bold")
        )
        self.engine_label.pack(side=tk.LEFT)

        hint_label = tk.Label(
            top_frame,
            text="Enter=貼り付け | Ctrl+Enter=コピー | Tab=エンジン切替 | Esc=閉じる",
            bg=bg, fg="#6c7086",
            font=("Segoe UI", 8)
        )
        hint_label.pack(side=tk.RIGHT)

        # Input field
        fsize = self.fsize
        self.input_var = tk.StringVar()
        self.input_entry = tk.Entry(
            main_frame,
            textvariable=self.input_var,
            bg=input_bg, fg=fg, insertbackground=fg,
            font=("Segoe UI", fsize),
            relief=tk.FLAT, bd=0,
            highlightthickness=2, highlightcolor=accent, highlightbackground="#45475a"
        )
        self.input_entry.pack(fill=tk.X, ipady=6)
        self.input_entry.focus_set()

        # Result label
        self.result_var = tk.StringVar(value="")
        self.result_label = tk.Label(
            main_frame,
            textvariable=self.result_var,
            bg=bg, fg=result_fg,
            font=("Segoe UI", fsize),
            anchor="w", justify="left",
            wraplength=self.min_w - 40
        )
        self.result_label.pack(fill=tk.X, pady=(8, 0))

        # Bindings
        self.input_var.trace_add("write", self._on_input_change)
        self.root.bind("<Escape>", lambda e: self._close())
        self.root.bind("<Return>", self._on_enter)
        self.root.bind("<Control-Return>", self._on_ctrl_enter)
        self.root.bind("<Tab>", self._toggle_engine)
        self.root.bind("<FocusOut>", self._on_focus_out)

        # If initial text provided, set it
        if initial_text:
            self.input_var.set(initial_text)

    def _on_input_change(self, *args):
        """Debounced translation on input change."""
        if self.translate_timer:
            self.root.after_cancel(self.translate_timer)
        self.translate_timer = self.root.after(400, self._do_translate)

    def _resize_to_content(self, input_text: str, result_text: str):
        """Dynamically resize window based on content length."""
        try:
            # Estimate character width (rough: 1 CJK char ≈ 2 latin chars)
            def effective_len(s):
                n = 0
                for ch in s:
                    if '\u3000' <= ch <= '\u9FFF' or '\uF900' <= ch <= '\uFAFF':
                        n += 2
                    else:
                        n += 1
                return n

            longer = max(effective_len(input_text), effective_len(result_text))
            char_px = self.fsize * 0.65  # approx pixels per char

            # Width: scale with text length
            need_w = int(longer * char_px) + 80  # padding
            new_w = max(self.min_w, min(need_w, self.max_w))

            # Height: count wrapped lines for result
            wrap_chars = max(1, int((new_w - 40) / char_px))
            result_lines = max(1, -(-effective_len(result_text) // wrap_chars))  # ceil div
            # Also count newlines
            result_lines += result_text.count('\n')
            line_h = self.fsize + 10
            # Top bar ~30 + input ~40 + padding ~40 + result lines
            need_h = 30 + 40 + 40 + int(result_lines * line_h)
            new_h = max(self.min_h, min(need_h, self.max_h))

            # Update wraplength
            self.result_label.config(wraplength=new_w - 40)

            # Reposition centered
            x = (self.screen_w - new_w) // 2
            y = (self.screen_h - new_h) // 3
            self.root.geometry(f"{new_w}x{new_h}+{x}+{y}")
        except Exception:
            pass

    def _do_translate(self):
        text = self.input_var.get().strip()
        if not text:
            self.result_var.set("")
            return
        self.result_var.set("翻訳中...")

        def _translate():
            try:
                result, target = do_translate(text, self.config)
                def _update():
                    self.result_var.set(result)
                    self._resize_to_content(text, result)
                self.root.after(0, _update)
                self._last_result = result
                self._last_target = target
            except Exception as e:
                self.root.after(0, lambda: self.result_var.set(f"Error: {e}"))
                self._last_result = ""
                self._last_target = ""

        self._last_result = ""
        self._last_target = ""
        threading.Thread(target=_translate, daemon=True).start()

    def _on_enter(self, event):
        """Enter: copy result to clipboard and paste into active window."""
        result = getattr(self, "_last_result", "")
        if result:
            self._copy_and_paste(result)

    def _on_ctrl_enter(self, event):
        """Ctrl+Enter: copy result to clipboard only."""
        result = getattr(self, "_last_result", "")
        if result:
            set_clipboard(result)
            self._log_and_close(result)

    def _copy_and_paste(self, text: str):
        """Copy to clipboard and simulate Ctrl+V paste."""
        set_clipboard(text)
        self._close()
        # Small delay then paste
        time.sleep(0.15)
        try:
            import pyautogui
            pyautogui.hotkey("ctrl", "v")
        except ImportError:
            # Fallback: just copy to clipboard, user can paste manually
            pass

    def _toggle_engine(self, event):
        """Toggle between Google and DeepL."""
        if self.current_engine == "google":
            if self.config.get("deepl_api_key"):
                self.current_engine = "deepl"
            else:
                self.result_var.set("DeepL APIキーが未設定です")
                return "break"
        else:
            self.current_engine = "google"
        self.config["engine"] = self.current_engine
        self.engine_label.config(text=f"⚡ {self.current_engine.upper()}")
        # Re-translate with new engine
        if self.input_var.get().strip():
            self._do_translate()
        return "break"

    def _on_focus_out(self, event):
        """Close when focus is lost (click outside)."""
        # Small delay to avoid closing on internal focus changes
        self.root.after(100, self._check_focus)

    def _check_focus(self):
        try:
            if not self.root.focus_get():
                self._close()
        except Exception:
            pass

    def _log_and_close(self, result: str):
        original = self.input_var.get().strip()
        target = getattr(self, "_last_target", "")
        log_translation(original, result, self.current_engine, target, self.config)
        self._close()

    def _close(self):
        # Log if there's a result
        result = getattr(self, "_last_result", "")
        if result:
            original = self.input_var.get().strip()
            target = getattr(self, "_last_target", "")
            log_translation(original, result, self.current_engine, target, self.config)
        try:
            self.root.destroy()
        except Exception:
            pass
        if self.on_close:
            self.on_close()

    def run(self):
        self.root.mainloop()


# ---------------------------------------------------------------------------
# System Tray (using pystray)
# ---------------------------------------------------------------------------
class SystemTrayApp:
    """System tray icon with menu."""

    def __init__(self):
        self.config = load_config()
        self.popup_open = False

    def _create_icon_image(self):
        """Create a simple icon using PIL."""
        from PIL import Image, ImageDraw, ImageFont
        img = Image.new("RGBA", (64, 64), (30, 30, 46, 255))
        draw = ImageDraw.Draw(img)
        # Draw "翻" character or "T" as icon
        try:
            fnt = ImageFont.truetype("segoeui.ttf", 36)
        except Exception:
            fnt = ImageFont.load_default()
        draw.text((14, 10), "T", fill=(137, 180, 250, 255), font=fnt)
        return img

    def show_popup(self, initial_text: str = ""):
        if self.popup_open:
            return
        self.popup_open = True

        def on_close():
            self.popup_open = False

        def _run():
            popup = TranslatePopup(self.config, initial_text=initial_text, on_close=on_close)
            popup.run()

        threading.Thread(target=_run, daemon=True).start()

    def translate_selected(self):
        """Get selected text from clipboard and translate."""
        import pyautogui
        # Save current clipboard
        old_clip = get_clipboard()
        # Copy selection
        pyautogui.hotkey("ctrl", "c")
        time.sleep(0.15)
        selected = get_clipboard()
        # Restore old clipboard
        if old_clip:
            set_clipboard(old_clip)
        if selected:
            self.show_popup(initial_text=selected)

    def open_config(self):
        """Open config file in default editor."""
        os.startfile(str(CONFIG_FILE))

    def open_log(self):
        """Open translation log."""
        if LOG_FILE.exists():
            os.startfile(str(LOG_FILE))

    def toggle_engine(self):
        if self.config["engine"] == "google":
            self.config["engine"] = "deepl"
        else:
            self.config["engine"] = "google"
        save_config(self.config)

    def run(self):
        import pystray
        from pystray import MenuItem as Item

        icon_image = self._create_icon_image()

        def engine_text(item=None):
            return f"エンジン: {self.config['engine'].upper()}"

        menu = pystray.Menu(
            Item("翻訳ポップアップ", lambda: self.show_popup()),
            Item("選択テキスト翻訳", lambda: self.translate_selected()),
            pystray.Menu.SEPARATOR,
            Item(engine_text, lambda: self.toggle_engine(), default=False),
            Item("設定ファイルを開く", lambda: self.open_config()),
            Item("翻訳ログを開く", lambda: self.open_log()),
            pystray.Menu.SEPARATOR,
            Item("終了", lambda icon, item: icon.stop()),
        )

        self.icon = pystray.Icon("QuickTranslate", icon_image, "Quick Translate", menu)

        # Register global hotkeys in background
        self._register_hotkeys()

        self.icon.run()

    def _register_hotkeys(self):
        """Register global hotkeys using keyboard library."""
        def _hotkey_thread():
            try:
                import keyboard
                hk_popup = self.config.get("hotkey_popup", "ctrl+shift+t")
                hk_selected = self.config.get("hotkey_selected", "ctrl+shift+y")
                keyboard.add_hotkey(hk_popup, lambda: self.show_popup())
                keyboard.add_hotkey(hk_selected, lambda: self.translate_selected())
                keyboard.wait()  # Block forever
            except ImportError:
                print("Warning: 'keyboard' module not found. Global hotkeys disabled.", file=sys.stderr)
                print("Install with: pip install keyboard", file=sys.stderr)
            except Exception as e:
                print(f"Hotkey error: {e}", file=sys.stderr)

        t = threading.Thread(target=_hotkey_thread, daemon=True)
        t.start()


# ---------------------------------------------------------------------------
# CLI Mode (for AutoHotkey integration)
# ---------------------------------------------------------------------------
def cli_translate(text: str, config: dict):
    """Translate text and print result (for AHK integration)."""
    result, target = do_translate(text, config)
    print(result)
    set_clipboard(result)
    log_translation(text, result, config.get("engine", "google"), target, config)


def cli_selected(config: dict):
    """Translate currently selected text via clipboard."""
    import pyautogui
    old_clip = get_clipboard()
    pyautogui.hotkey("ctrl", "c")
    time.sleep(0.15)
    selected = get_clipboard()
    if old_clip:
        set_clipboard(old_clip)
    if selected:
        popup = TranslatePopup(config, initial_text=selected)
        popup.run()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def main():
    parser = argparse.ArgumentParser(description="Quick Translate for Windows")
    parser.add_argument("--translate", "-t", type=str, help="Translate text directly")
    parser.add_argument("--popup", "-p", action="store_true", help="Show popup window")
    parser.add_argument("--popup-file", type=str, help="Show popup with text from file")
    parser.add_argument("--selected", "-s", action="store_true", help="Translate selected text")
    parser.add_argument("--engine", "-e", choices=["google", "deepl"], help="Override engine")
    args = parser.parse_args()

    config = load_config()
    if args.engine:
        config["engine"] = args.engine

    if args.translate:
        cli_translate(args.translate, config)
    elif args.popup_file:
        initial = ""
        try:
            with open(args.popup_file, "r", encoding="utf-8") as f:
                initial = f.read().strip()
            os.remove(args.popup_file)
        except Exception:
            pass
        popup = TranslatePopup(config, initial_text=initial)
        popup.run()
    elif args.popup:
        popup = TranslatePopup(config)
        popup.run()
    elif args.selected:
        cli_selected(config)
    else:
        # Default: run system tray app
        app = SystemTrayApp()
        app.run()


if __name__ == "__main__":
    main()
