//! RTF bodies as Outlook keeps them, for the tests here and those of the crates that read
//! Outlook's files (the `test-support` feature): one copy of each, so a fix to one is made once.

/// MS-OXRTFCP 3.1.1: the specification's compressed RTF (LZFu), which comes out as
/// `{\rtf1\ansi\ansicpg1252\pard hello world}` and a line break.
pub const SPEC_LZFU: [u8; 49] = [
    0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5, 0xc7, 0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63, 0x70, 0x67,
    0x31, 0x32, 0x35, 0x42, 0x32, 0x0a, 0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0, 0x6c, 0x64, 0x7d, 0x0a, 0x80, 0x0f,
    0xa0,
];

/// HTML wrapped in RTF as Outlook writes it (MS-OXRTFEX): a bold word and a picture named by
/// `cid:logo@studio.example`.
pub const STORYBOARD_RTF: &[u8] = br#"{\rtf1\ansi\ansicpg1252\fromhtml1 \deff0{\fonttbl{\f0\fswiss Calibri;}}{\*\htmltag19 <html>}{\*\htmltag34 <body>}{\*\htmltag <p>}\htmlrtf {\htmlrtf0 Hi Riley, the {\*\htmltag <b>}\htmlrtf\b\htmlrtf0 storyboard{\*\htmltag </b>}\htmlrtf\b0\htmlrtf0  is attached.{\*\htmltag <img src="cid:logo@studio.example">}\htmlrtf\par\htmlrtf0}{\*\htmltag </p></body></html>}}"#;

/// `PidTagRtfCompressed` stored as it is (`MELA`), as Outlook may keep a short body.
pub fn stored_rtf(rtf: &[u8]) -> Vec<u8> {
    let len = u32::try_from(rtf.len()).unwrap_or(u32::MAX);
    let mut out = len.saturating_add(12).to_le_bytes().to_vec();
    out.extend(len.to_le_bytes());
    out.extend(0x414C_454Du32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(rtf);
    out
}
