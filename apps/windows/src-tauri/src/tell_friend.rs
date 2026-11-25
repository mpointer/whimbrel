//! Tell a friend from the tray's right-click menu (docs/superpowers/plans/2026-11-24-referral-and-share.md, rollout row 1).
//!
//! The item copies a short message with a plain link to whimbrelapp.com. No account is needed, no code and no reward is
//! part of the message, nothing opens and nothing is sent. The popup and Settings do the same through the web clipboard
//! (`ui/src/share/tellAFriend.ts`); a native menu cannot reach `navigator.clipboard`, so this side writes the Windows
//! clipboard itself, with the Win32 calls the snapshot copy already uses (no new dependency).

use std::time::Duration;

use crate::l10n::tr;

/// The plain link: the home page with the one campaign tag the site already keeps in the visitor's own browser. The UI holds the
/// same link (`TELL_FRIEND_LINK` in `ui/src/share/tellAFriend.ts`), and a test on each side pins it.
pub const LINK: &str = "https://whimbrelapp.com/?utm_source=share";

/// The message copied, in the shell's language. A translation that lost its `{link}` still gives a message with the link in it
/// once, at the end, so a copy never loses the link.
pub fn message() -> String {
    with_link(&tr("I use Whimbrel for my meeting notes. It records without a bot, and my recordings stay on my computer. Free for Mac and Windows: {link}", &[("link", LINK)]))
}

/// `text` with the link in it exactly where the translation put it, or appended when it is missing.
fn with_link(text: &str) -> String {
    if text.contains(LINK) {
        text.to_string()
    } else {
        format!("{} {}", text.replace("{link}", "").trim(), LINK).trim().to_string()
    }
}

/// Puts `text` on the clipboard as Unicode text. Another app can hold the clipboard for a moment, so opening it is retried
/// briefly.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    // UTF-16 with the closing NUL the format needs.
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();
    if !(0..5).any(|attempt| {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { OpenClipboard(None).is_ok() }
    }) {
        return Err(tr("Another app is using the clipboard. Try again in a moment.", &[]));
    }
    let result = unsafe {
        (|| {
            EmptyClipboard().map_err(|e| e.to_string())?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| e.to_string())?;
            let target = GlobalLock(memory);
            if target.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err("couldn't lock the clipboard memory".to_string());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target.cast::<u16>(), wide.len());
            let _ = GlobalUnlock(memory);
            // Once this succeeds the clipboard owns the memory, so it's never freed here.
            if let Err(e) = SetClipboardData(CF_UNICODETEXT.0.into(), Some(HANDLE(memory.0))) {
                let _ = GlobalFree(Some(memory));
                return Err(e.to_string());
            }
            Ok(())
        })()
    };
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

/// Copies the message. Runs off the event loop: a busy clipboard is retried with short sleeps.
pub fn copy_message() -> Result<(), String> {
    copy_to_clipboard(&message())
}

/// What the toast says after a copy: that it worked, or that it did not.
pub fn toast_text(copied: &Result<(), String>) -> String {
    match copied {
        Ok(()) => tr("Message copied. Paste it wherever you like.", &[]),
        Err(_) => tr("Could not copy. Try again.", &[]),
    }
}

/// `text` as the body of a plain one-line toast, with the characters XML reserves written out.
pub fn toast_xml(text: &str) -> String {
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
    format!("<toast><visual><binding template=\"ToastGeneric\"><text>{escaped}</text></binding></visual></toast>")
}

/// Shows a Windows toast for `aumid`, the app's AppUserModelID. The installer's Start Menu shortcut carries it (`nsis/hooks.nsh`),
/// which a toast from an unpackaged app needs (see `alert.rs`). A build run from a folder without that shortcut shows nothing.
pub fn show_toast(aumid: &str, text: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    let doc = XmlDocument::new().map_err(|e| e.to_string())?;
    doc.LoadXml(&HSTRING::from(toast_xml(text))).map_err(|e| e.to_string())?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid))
        .and_then(|notifier| notifier.Show(&toast))
        .map_err(|e| e.to_string())
}

/// The tray item: copies the message, then says in a toast whether it worked.
pub fn copy_and_notify(aumid: &str) {
    let copied = copy_message();
    if let Err(e) = &copied {
        tracing::warn!(error = %e, "copy the Tell a friend message from the tray");
    }
    if let Err(e) = show_toast(aumid, &toast_text(&copied)) {
        tracing::warn!(error = %e, "show the Tell a friend toast");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_is_one_plain_line_with_the_link_once_and_no_code() {
        let text = message();
        assert_eq!(text.matches(LINK).count(), 1, "{text}");
        assert!(text.ends_with(LINK), "{text}");
        let lower = text.to_lowercase();
        assert!(!lower.contains("code") && !lower.contains("reward") && !lower.contains("discount"), "{text}");
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "{text}");
        assert!(!text.contains("{link}"), "{text}");
    }

    #[test]
    fn the_message_says_what_mike_chose() {
        assert_eq!(
            message(),
            "I use Whimbrel for my meeting notes. It records without a bot, and my recordings stay on my computer. Free for Mac and Windows: https://whimbrelapp.com/?utm_source=share"
        );
    }

    #[test]
    fn the_toast_says_whether_the_copy_worked() {
        assert_eq!(toast_text(&Ok(())), "Message copied. Paste it wherever you like.");
        assert_eq!(toast_text(&Err("busy".to_string())), "Could not copy. Try again.");
    }

    #[test]
    fn the_toast_xml_holds_one_line_of_escaped_text() {
        assert_eq!(
            toast_xml("Tom & \"Jo\" <ok>"),
            "<toast><visual><binding template=\"ToastGeneric\"><text>Tom &amp; &quot;Jo&quot; &lt;ok&gt;</text></binding></visual></toast>"
        );
        assert!(toast_xml("Message copied. Paste it wherever you like.").contains("<text>Message copied. Paste it wherever you like.</text>"));
    }

    #[test]
    fn the_link_is_the_plain_home_page_with_the_one_campaign_tag() {
        assert_eq!(LINK, "https://whimbrelapp.com/?utm_source=share");
    }

    #[test]
    fn a_translation_that_lost_its_placeholder_still_carries_the_link_once() {
        assert_eq!(with_link("Uso Whimbrel."), format!("Uso Whimbrel. {LINK}"));
        assert_eq!(with_link(&format!("Mira {LINK} ahora.")), format!("Mira {LINK} ahora."));
        assert_eq!(with_link("{link}"), LINK);
        assert_eq!(with_link(""), LINK);
    }
}
