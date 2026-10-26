//! Attachment names from a message, before anything is written to disk or handed to another app:
//! [`sanitize_filename`] turns whatever the sender wrote into a plain name we can save under, and
//! [`runs_code`] says whether opening that file would start a program.

/// Longest name we keep, in characters (file systems allow 255 bytes; this leaves room for UTF-8
/// and for a ` (2)` added on a clash).
const LONGEST_NAME: usize = 180;

/// What is saved when nothing usable is left of the sender's name.
const FALLBACK_NAME: &str = "attachment";

/// Makes the sender's file name safe to save under. Only the part after the last `/` or `\` is
/// kept, so the name can never point into another folder. Control characters and the ones Windows
/// refuses in names become `_`. Characters that disguise a name are dropped: a right-to-left
/// override turns `invoice\u{202E}fdp.exe` into what looks like `invoiceexe.pdf`. A name Windows
/// reserves for a device (`CON`, `nul.txt`, `COM1`) gets a leading `_`, since no such file can be
/// created there. An empty result becomes `attachment`.
pub fn sanitize_filename(raw: &str) -> String {
    let last_part = raw.split(['/', '\\']).next_back().unwrap_or_default();
    let mapped: String = last_part.chars().filter_map(plain_char).collect();
    // Leading and trailing spaces and dots go; Windows drops them silently anyway.
    let mapped = mapped.trim();
    let mapped = mapped.trim_matches('.');
    let mapped = mapped.trim();
    let mut name: String = mapped.chars().take(LONGEST_NAME).collect();
    if reserved_by_windows(&name) {
        name.insert(0, '_');
    }
    if name.is_empty() {
        name.push_str(FALLBACK_NAME);
    }
    name
}

/// How one character of a name is kept: dropped (`None`) when it only changes how the name is
/// shown, `_` when a file system would refuse it, otherwise as it is.
fn plain_char(c: char) -> Option<char> {
    const REFUSED: &str = "<>:\"|?*";
    if disguises(c) {
        None
    } else if c.is_control() || REFUSED.contains(c) {
        Some('_')
    } else {
        Some(c)
    }
}

/// Characters with no glyph of their own that alter how the text around them reads.
fn disguises(c: char) -> bool {
    match c {
        // Zero-width space, non-joiner and joiner; left-to-right and right-to-left marks.
        '\u{200B}'..='\u{200F}' => true,
        // Bidirectional embeddings and overrides (LRE, RLE, PDF, LRO, RLO).
        '\u{202A}'..='\u{202E}' => true,
        // Bidirectional isolates (LRI, RLI, FSI, PDI).
        '\u{2066}'..='\u{2069}' => true,
        // Word joiner and the invisible operators.
        '\u{2060}'..='\u{2064}' => true,
        // Line and paragraph separators.
        '\u{2028}' | '\u{2029}' => true,
        // The byte-order mark, which inside a name is a zero-width no-break space.
        '\u{FEFF}' => true,
        // Arabic letter mark; Mongolian vowel separator.
        '\u{61C}' | '\u{180E}' => true,
        _ => false,
    }
}

/// Whether Windows keeps this name for a device. Only the part before the first dot counts, so
/// `prn.txt` and `COM1.log` are taken too, while `Console.txt` and `COM10.txt` are not.
fn reserved_by_windows(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    if ["AUX", "CON", "CONIN$", "CONOUT$", "NUL", "PRN"].contains(&base.as_str()) {
        return true;
    }
    // A serial or printer port: COM or LPT and one digit, 0 to 9, or a superscript ¹ ² ³, which
    // Windows reserves too (craft-libs' file names found these).
    let port = base.strip_prefix("COM").or_else(|| base.strip_prefix("LPT"));
    let mut digit = port.unwrap_or_default().chars();
    port.is_some() && matches!((digit.next(), digit.next()), (Some('0'..='9' | '¹' | '²' | '³'), None))
}

/// Extensions whose files start code when opened, grouped by where they do it.
const EXECUTABLE_EXTENSIONS: &[&[&str]] = &[
    // Windows: programs and the things that load into them.
    &["exe", "com", "pif", "scr", "dll", "ocx", "cpl", "msc", "sys", "drv", "gadget", "xll"],
    // Windows: installers, app packages and click-to-run deployments.
    &["msi", "msp", "mst", "msix", "msixbundle", "appx", "appxbundle", "appinstaller"],
    &["application", "appref-ms", "xbap", "vsix"],
    // Windows: batch files, PowerShell and the Windows Script Host.
    &["bat", "cmd", "ps1", "psm1", "psd1", "ps1xml", "psc1"],
    &["vb", "vbs", "vbe", "js", "jse", "wsf", "wsc", "wsh", "hta", "sct"],
    // Windows: registry and setup files, help files, Access databases with code in them.
    &["reg", "inf", "ins", "isp", "chm", "hlp"],
    &["mde", "mda", "mdz", "ade", "adp", "accde"],
    // Windows: shortcuts, links and shell files that launch something.
    &["lnk", "url", "website", "scf", "library-ms", "searchconnector-ms", "settingcontent-ms", "diagcab"],
    // Windows: disk images that mount and run.
    &["iso", "img", "vhd", "vhdx"],
    // macOS: app bundles, disk images, installers, AppleScript and Automator.
    &["app", "dmg", "pkg", "mpkg", "applescript", "scpt", "workflow", "action", "command", "tool"],
    // Linux and other Unix systems: packages, self-running images, launchers and shells.
    &["deb", "rpm", "appimage", "run", "bin", "desktop", "sh", "bash", "zsh", "csh", "ksh"],
    // Anywhere: interpreters and runtimes that an operating system may set up to open.
    &["jar", "apk", "py", "pyw", "pl", "rb", "php", "lua"],
];

/// Whether opening this attachment would run code: a program, script, installer, shortcut, disk
/// image and so on. MailCraft never asks another app to open such a file; the user may still save
/// it. As with the operating system, only the last extension decides, so `report.pdf.exe` counts.
pub fn runs_code(name: &str) -> bool {
    let cleaned = sanitize_filename(name).to_ascii_lowercase();
    let Some((_, extension)) = cleaned.rsplit_once('.') else { return false };
    let extension = extension.trim();
    EXECUTABLE_EXTENSIONS.iter().any(|group| group.contains(&extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths, refused characters, disguising characters and device names all come out plain;
    /// ordinary names, accents and dashes included, are left alone.
    #[test]
    fn sender_names_become_plain_file_names() {
        let cases = [
            ("../tmp\\x/../q|r*s.doc", "q_r_s.doc"),
            ("C:\\Users\\pat\\bed<3>.csv", "bed_3_.csv"),
            (". .", "attachment"),
            ("", "attachment"),
            ("photo\u{202E}gpj.scr", "photogpj.scr"),
            ("\u{200D}plan\u{2060}s\u{FEFF}.odt", "plans.odt"),
            ("\u{2067}\u{2068}\u{180E}", "attachment"),
            ("aux", "_aux"),
            ("Prn.html", "_Prn.html"),
            ("LPT9.dat", "_LPT9.dat"),
            ("conout$", "_conout$"),
            ("Auxiliary.txt", "Auxiliary.txt"),
            ("LPT0.txt", "_LPT0.txt"),
            ("com¹.txt", "_com¹.txt"),
            ("LPT³", "_LPT³"),
            ("COM⁴.txt", "COM⁴.txt"),
            ("COM12", "COM12"),
            ("Ångström – notes 3.md", "Ångström – notes 3.md"),
        ];
        for (raw, want) in cases {
            assert_eq!(sanitize_filename(raw), want, "{raw:?}");
        }
        let long = "y".repeat(400) + ".txt";
        assert_eq!(sanitize_filename(&long).chars().count(), LONGEST_NAME);
    }

    /// Whatever a name pretends to be first, its last extension says whether it runs.
    #[test]
    fn programs_and_scripts_are_told_from_documents() {
        let run = [
            "Installer.EXE",
            "minutes.docx.exe",
            "Minutes.DOCX.Exe  ",
            "photo\u{202E}gpj.scr",
            "go.cmd",
            "tidy.bat",
            "fix.ps1",
            "help.hta",
            "Report.lnk",
            "link.url",
            "image.iso",
            "Tool.app",
            "game.jar",
            "macro.vbs",
            "loader.js",
            "update.msi",
            "plugin.xll",
        ];
        for name in run {
            assert!(runs_code(name), "{name} should count as code");
        }
        let safe = [
            "minutes.docx",
            "sunset.png",
            "readme.txt",
            "sums.ods",
            "talk.odp",
            "bundle.tar.gz",
            "msi",
            "the.exe.plan.md",
            "meeting.ics",
            "pat.vcf",
            "saved.eml",
            "",
        ];
        for name in safe {
            assert!(!runs_code(name), "{name} should not count as code");
        }
    }
}
