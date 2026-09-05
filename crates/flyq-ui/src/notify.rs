//! Best-effort desktop notifications.
//!
//! LanChat raises a system notification for three events (matching the classic
//! FeiQ behaviour): an incoming chat message, a peer offering file(s), and a
//! completed file download. Notifications are purely informational — the same
//! information is always visible in-app — so any failure is swallowed and logged
//! at debug level.
//!
//! The OS call is dispatched onto a Tokio blocking worker so it never runs on
//! GPUI's UI thread. On Windows `notify-rust` shows a WinRT toast; it defaults
//! to the well-known PowerShell AppUserModelID, which is the reliable choice for
//! an unpackaged executable (a custom, unregistered AUMID would make toasts
//! silently disappear).

use notify_rust::Notification;
use tokio::runtime::Handle;

/// Source name shown on the notification (used on Linux/macOS; Windows toasts
/// are attributed via the AppUserModelID instead).
pub const APP_NAME: &str = "LanChat";

/// Maximum number of characters shown from a message body / filename before the
/// preview is truncated with an ellipsis.
const PREVIEW_CHARS: usize = 120;

/// Collapse runs of whitespace to single spaces and truncate `body` to at most
/// `max_chars` characters, appending an ellipsis when truncated.
///
/// Counting is by `char` (not bytes) so CJK text is never split mid-codepoint.
fn preview(body: &str, max_chars: usize) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let head: String = collapsed.chars().take(max_chars).collect();
    format!("{}…", head.trim_end())
}

/// The body text for a "files offered" notification, worded for one vs. many.
fn offer_body(count: usize) -> String {
    match count {
        1 => "对方发来 1 个文件，等待接收".to_string(),
        n => format!("对方发来 {} 个文件，等待接收", n),
    }
}

/// Show a notification on a blocking worker. Failures are non-fatal.
fn push(rt: &Handle, summary: String, body: String) {
    rt.spawn_blocking(move || {
        if let Err(e) = Notification::new()
            .summary(&summary)
            .body(&body)
            .appname(APP_NAME)
            .show()
        {
            tracing::debug!("Desktop notification failed: {}", e);
        }
    });
}

/// Notify about an incoming chat message from `sender` with the given body.
pub fn message(rt: &Handle, sender: &str, content: &str) {
    push(
        rt,
        format!("{} 发来消息", sender),
        preview(content, PREVIEW_CHARS),
    );
}

/// Notify that `name` offered `count` file(s)/folder(s) for download.
pub fn file_offer(rt: &Handle, name: &str, count: usize) {
    push(rt, format!("{} 发来文件", name), offer_body(count));
}

/// Notify that a file finished downloading.
pub fn file_complete(rt: &Handle, filename: &str) {
    push(
        rt,
        "文件接收完成".to_string(),
        preview(filename, PREVIEW_CHARS),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_keeps_short_text_verbatim() {
        assert_eq!(preview("hello world", PREVIEW_CHARS), "hello world");
    }

    #[test]
    fn preview_collapses_whitespace() {
        assert_eq!(preview("  a \n\t b   c  ", PREVIEW_CHARS), "a b c");
    }

    #[test]
    fn preview_truncates_long_ascii_with_ellipsis() {
        let body = "x".repeat(PREVIEW_CHARS + 30);
        let out = preview(&body, PREVIEW_CHARS);
        assert_eq!(out.chars().count(), PREVIEW_CHARS + 1); // + ellipsis
        assert!(out.ends_with('…'));
    }

    #[test]
    fn preview_counts_cjk_by_char_not_byte() {
        // Each CJK char is 3 bytes; char-based truncation must not panic or
        // split a codepoint.
        let body = "测".repeat(PREVIEW_CHARS + 10);
        let out = preview(&body, PREVIEW_CHARS);
        assert_eq!(out.chars().count(), PREVIEW_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn preview_returns_empty_for_blank_input() {
        assert_eq!(preview("   \n  ", PREVIEW_CHARS), "");
    }

    #[test]
    fn offer_body_is_singular_for_one() {
        assert_eq!(offer_body(1), "对方发来 1 个文件，等待接收");
    }

    #[test]
    fn offer_body_is_plural_for_many() {
        assert_eq!(offer_body(3), "对方发来 3 个文件，等待接收");
    }

    /// Manual verification: actually raises a real OS notification. Ignored by
    /// default because it has a visible side effect; run it with
    /// `cargo test -p flyq-ui -- --ignored --nocapture` to confirm the platform
    /// notification path succeeds (on Windows this exercises the WinRT toast).
    #[test]
    #[ignore = "shows a real OS notification; run manually"]
    fn manual_show_real_notification() {
        let result = Notification::new()
            .summary("LanChat 通知自检")
            .body("如果你看到这条通知，说明系统通知可用。")
            .appname(APP_NAME)
            .show();
        assert!(
            result.is_ok(),
            "notification show() failed: {:?}",
            result.err()
        );
    }
}
