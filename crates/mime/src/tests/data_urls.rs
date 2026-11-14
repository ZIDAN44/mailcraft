//! A picture written into HTML as `data:` read by one reader ([`crate::attachments::data_url`])
//! where there were two: a reply's quoted pictures and a signature's. Each old one is kept here,
//! as it was, to show the one gives what each gave.

use super::*;
use proptest::prelude::*;

/// A reply's quoted pictures' cap (`attachments::QUOTED_PICTURE_MAX`).
const QUOTED_MAX: usize = 25 << 20;

/// The quoted pictures' reader, as it was.
fn old_quoted(src: &str) -> Option<(String, Vec<u8>)> {
    let rest = src.trim().get(..11).filter(|p| p.eq_ignore_ascii_case("data:image/")).and_then(|_| src.trim().get(5..))?;
    let (kind, b64) = rest.split_once(',')?;
    let mime = kind.strip_suffix(";base64").or_else(|| kind.strip_suffix(";BASE64"))?.to_ascii_lowercase();
    if !["image/png", "image/jpeg", "image/jpg", "image/gif", "image/webp", "image/bmp"].contains(&mime.as_str())
        || b64.len() > QUOTED_MAX / 3 * 4 + 8
    {
        return None;
    }
    let clean: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let data = encodify::base64::LENIENT.decode(&clean).ok().filter(|d| !d.is_empty())?;
    Some((if mime == "image/jpg" { "image/jpeg".into() } else { mime }, data))
}

/// The signature's (in `data_pictures`), as it was: the extension its picture is named with.
fn old_signature(src: &str) -> Option<(&'static str, Vec<u8>)> {
    let rest = src.trim().get(..11).filter(|p| p.eq_ignore_ascii_case("data:image/")).and_then(|_| src.trim().get(11..))?;
    let (kind, b64) = rest.split_once(',')?;
    let kind = kind.to_ascii_lowercase();
    let ext = match kind.as_str() {
        "png;base64" => "png",
        "jpeg;base64" | "jpg;base64" => "jpg",
        "gif;base64" => "gif",
        "webp;base64" => "webp",
        _ => return None,
    };
    if b64.len() > SIGNATURE_PICTURE_MAX / 3 * 4 + 8 {
        return None;
    }
    let clean: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let data = encodify::base64::LENIENT.decode(&clean).ok().filter(|d| !d.is_empty() && d.len() <= SIGNATURE_PICTURE_MAX)?;
    Some((ext, data))
}

/// The signature's as it reads one now.
fn signature(src: &str) -> Option<(&'static str, Vec<u8>)> {
    let (mime, data) = crate::attachments::data_url(src, SIGNATURE_PICTURE_MAX)?;
    SIGNATURE_PICTURE_TYPES.contains(&mime.as_str()).then(|| (crate::files::ext_for_mime(&mime).trim_start_matches('.'), data))
}

/// Addresses made of what a `data:` address is made of, in its order (mostly well formed), or
/// its pieces in any order.
fn sources() -> impl Strategy<Value = String> {
    let parts = (
        prop_oneof![Just(" data:"), Just("data:"), Just("DATA:"), Just("data:text/"), Just("")],
        prop_oneof![Just("image/"), Just("IMAGE/"), Just("image")],
        prop_oneof![Just("png"), Just("PNG"), Just("jpeg"), Just("jpg"), Just("gif"), Just("webp"), Just("bmp"), Just("svg+xml"), Just("tiff")],
        prop_oneof![Just(";base64"), Just(";BASE64"), Just(";charset=x"), Just("")],
        prop_oneof![Just(","), Just(", "), Just("")],
        prop_oneof!["[A-Za-z0-9+/= \n]{0,24}", Just(PNG_1X1.to_string()), Just(format!("{} \n{}", &PNG_1X1[..40], &PNG_1X1[40..]))],
    );
    let ordered = parts.clone().prop_map(|(a, b, c, d, e, f)| format!("{a}{b}{c}{d}{e}{f} "));
    let shuffled = parts.prop_map(|(a, b, c, d, e, f)| format!("{f}{e}{a}{d}{c}{b}"));
    prop_oneof![4 => ordered, 1 => shuffled]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]

    #[test]
    fn one_reader_gives_what_each_gave(src in sources()) {
        prop_assert_eq!(crate::attachments::data_url(&src, QUOTED_MAX), old_quoted(&src));
        prop_assert_eq!(signature(&src), old_signature(&src));
    }
}

/// The addresses tried are read as pictures often enough to compare what's read.
#[test]
fn the_addresses_tried_are_often_pictures() {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let mut read = 0;
    for _ in 0..1000 {
        let src = sources().new_tree(&mut runner).map(|t| proptest::strategy::ValueTree::current(&t)).unwrap_or_default();
        read += usize::from(old_quoted(&src).is_some());
    }
    assert!(read > 50, "{read} of 1000");
}

/// Whole addresses, and the one change: a `;Base64` in mixed case is read in a reply too, as a
/// signature read it.
#[test]
fn pictures_written_as_data_are_read() {
    let png = format!("data:image/png;base64,{PNG_1X1}");
    assert_eq!(signature(&png).map(|(e, d)| (e, d.len())), old_signature(&png).map(|(e, d)| (e, d.len())));
    assert!(signature(&png).is_some());
    let mixed = format!("data:image/png;Base64,{PNG_1X1}");
    assert_eq!(old_quoted(&mixed), None);
    assert!(crate::attachments::data_url(&mixed, QUOTED_MAX).is_some_and(|(m, _)| m == "image/png"));
    assert!(signature(&mixed).is_some() && old_signature(&mixed).is_some());
    // A picture bigger than its cap isn't read.
    assert_eq!(crate::attachments::data_url(&png, 10), None);
}
