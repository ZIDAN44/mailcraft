//! The words of the system's file dialogs MailCraft opens, their titles and the kinds of file
//! they offer, in the window's language (issue #292: "Save as" and "Email message (*.eml)" were
//! English in a German window). The desktop app opens the dialogs and asks for them here.

/// A file dialog's title or a kind of file it offers, in the window's language: `english` as
/// the desktop app names it (one this doesn't know comes back as it is).
pub fn file_dialog_word(english: &str) -> String {
    let word = match english {
        "Attach files" => tl!("Attach files"),
        "Save as" => tl!("Save as"),
        "Save attachments to" => tl!("Save attachments to"),
        "Import certificate" => tl!("Import certificate"),
        "Trust a certificate file" => tl!("Trust a certificate file"),
        "Import OpenPGP key" => tl!("Import OpenPGP key"),
        "Import rules" => tl!("Import rules"),
        "Import categories" => tl!("Import categories"),
        "Open data file" => tl!("Open data file"),
        "Open mail file" => tl!("Open mail file"),
        "Insert pictures" => tl!("Insert pictures"),
        "All files" => tl!("All files"),
        "Certificate with its key" => tl!("Certificate with its key"),
        "Certificate" => tl!("Certificate"),
        "OpenPGP key" => tl!("OpenPGP key"),
        "MailCraft rules" => tl!("MailCraft rules"),
        "MailCraft categories" => tl!("MailCraft categories"),
        "Data files" => tl!("Data files"),
        "Outlook data files" => tl!("Outlook data files"),
        "MailCraft data files" => tl!("MailCraft data files"),
        "Mail files" => tl!("Mail files"),
        "Pictures" => tl!("Pictures"),
        other => other,
    };
    word.to_string()
}

/// The kind of file a name ending in `.ext` is, as a dialog that saves one offers it (Save as
/// type), in the window's language.
pub fn file_kind(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "pst" => tl!("Outlook data file").to_string(),
        "eml" => tl!("Email message").to_string(),
        "pdf" => tl!("PDF document").to_string(),
        "ics" => tl!("Calendar").to_string(),
        "vcf" => tl!("Contact card").to_string(),
        "txt" => tl!("Text").to_string(),
        "json" => tl!("MailCraft rules").to_string(),
        "html" | "htm" => tl!("Web page").to_string(),
        e => tf!("{ext} file", ext = e.to_ascii_uppercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, set_current};

    /// Every title and kind the desktop app asks for comes back in the window's language.
    #[test]
    fn a_file_dialog_speaks_the_windows_language() {
        set_current(Lang::from_code("de").expect("German"));
        assert_eq!(file_dialog_word("Save as"), "Speichern unter");
        assert_eq!(file_dialog_word("All files"), "Alle Dateien");
        assert_eq!(file_kind("EML"), tl!("Email message"));
        assert_eq!(file_kind("zip"), "ZIP-Datei");
        assert_eq!(file_dialog_word("Something else"), "Something else");
        set_current(Lang::EN);
        assert_eq!(file_dialog_word("Save as"), "Save as");
    }
}
