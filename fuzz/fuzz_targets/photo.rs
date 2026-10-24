//! A photo about to be sent: where it was taken is taken out of its EXIF and XMP blocks, and
//! what comes back is taken out of again (there's nothing left the second time).
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;

fuzz_target!(|input: &[u8]| {
    if let Some(clean) = mime::without_location(input) {
        assert!(mime::without_location(&clean).is_none(), "the place is still there");
    }
});
