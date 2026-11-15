//! What Outlook's data files and items say, and how: the ids, flags and values both the import
//! (`pst`, `pst_pim`) and the export (`pst_export`) read and write. Kept once, as the two must
//! agree to the byte (an exported file is read back to check it).

use mailcraft_model::ical::ShowAs;
use mailcraft_pst::GuidValue;

/// The end of most property sets' GUIDs.
pub(crate) const OLE: [u8; 8] = [0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// The property sets of named properties (MS-OXPROPS 1.3.2).
pub(crate) const PSETID_ADDRESS: GuidValue = GuidValue::new(0x0006_2004, 0, 0, OLE);
pub(crate) const PSETID_APPOINTMENT: GuidValue = GuidValue::new(0x0006_2002, 0, 0, OLE);
pub(crate) const PSETID_TASK: GuidValue = GuidValue::new(0x0006_2003, 0, 0, OLE);
pub(crate) const PSETID_NOTE: GuidValue = GuidValue::new(0x0006_200E, 0, 0, OLE);
pub(crate) const PSETID_COMMON: GuidValue = GuidValue::new(0x0006_2008, 0, 0, OLE);
pub(crate) const PSETID_LOG: GuidValue = GuidValue::new(0x0006_200A, 0, 0, OLE);
pub(crate) const PSETID_MEETING: GuidValue = GuidValue::new(0x6ED8_DA90, 0x450B, 0x101B, [0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05]);

/// PidLidGlobalObjectId's first bytes (MS-OXOCAL 2.2.1.27), and those of its Data for a meeting
/// whose uid isn't Outlook's, the uid following.
pub(crate) const GLOBAL_ID: [u8; 16] = [0x04, 0, 0, 0, 0x82, 0, 0xE0, 0, 0x74, 0xC5, 0xB7, 0x10, 0x1A, 0x82, 0xE0, 0x08];
pub(crate) const VCAL_UID: &[u8; 12] = b"vCal-Uid\x01\0\0\0";

/// Seconds from 1601-01-01 (a FILETIME's start) to 1970-01-01.
pub(crate) const EPOCH_1601: i64 = 11_644_473_600;
/// The same in minutes (a recurrence pattern's times).
pub(crate) const EPOCH_MINUTES: i64 = EPOCH_1601 / 60;

/// Unix seconds as a FILETIME: 100 ns since 1601 (0 before then).
pub(crate) fn filetime(unix: i64) -> i64 {
    unix.saturating_add(EPOCH_1601).saturating_mul(10_000_000).max(0)
}

/// When a series that doesn't end ends, as Outlook says it: 31 December 4500, 23:59, in minutes
/// from 1601 (a pattern's EndDate).
pub(crate) const NO_END: u32 = 0x5AE9_80DF;

/// ExceptionInfo's OverrideFlags (MS-OXOCAL 2.2.1.44.2): what a changed occurrence changed.
pub(crate) const ARO_SUBJECT: u16 = 0x0001;
pub(crate) const ARO_MEETINGTYPE: u16 = 0x0002;
pub(crate) const ARO_REMINDERDELTA: u16 = 0x0004;
pub(crate) const ARO_REMINDER: u16 = 0x0008;
pub(crate) const ARO_LOCATION: u16 = 0x0010;
pub(crate) const ARO_BUSYSTATUS: u16 = 0x0020;
pub(crate) const ARO_ATTACHMENT: u16 = 0x0040;
pub(crate) const ARO_SUBTYPE: u16 = 0x0080;
pub(crate) const ARO_APPTCOLOR: u16 = 0x0100;
pub(crate) const ARO_EXCEPTIONAL_BODY: u16 = 0x0200;
/// Weekdays as `BYDAY` names them, Sunday first (a pattern's bit 0).
pub(crate) const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

/// A contact group's members kept at most.
pub(crate) const MAX_MEMBERS: usize = 10_000;
/// A contact's phone fields by the kind MailCraft keeps (MS-OXOCNTC: business and its second,
/// home and its second, mobile, fax, pager, other), in the order they're read.
pub(crate) const PHONES: [(&str, &[u16]); 6] =
    [("work", &[0x3A08, 0x3A1B]), ("home", &[0x3A09, 0x3A2F]), ("cell", &[0x3A1C]), ("fax", &[0x3A24]), ("pager", &[0x3A21]), ("", &[0x3A1F])];
/// A contact's address fields (street, city, state, postal code, country) of business, home and
/// other.
pub(crate) const ADDRESSES: [(&str, [u16; 5]); 3] = [
    ("work", [0x3A29, 0x3A27, 0x3A28, 0x3A2A, 0x3A26]),
    ("home", [0x3A5D, 0x3A59, 0x3A5C, 0x3A5B, 0x3A5A]),
    ("", [0x3A63, 0x3A5F, 0x3A62, 0x3A61, 0x3A60]),
];

/// PidLidBusyStatus and how an event shows its time.
pub(crate) const BUSY: [(i32, ShowAs); 5] =
    [(0, ShowAs::Free), (1, ShowAs::Tentative), (2, ShowAs::Busy), (3, ShowAs::OutOfOffice), (4, ShowAs::WorkingElsewhere)];
/// PidTagRecipientTrackStatus and the guest's `PARTSTAT` (MS-OXOCAL 2.2.4.10).
pub(crate) const TRACK: [(i32, &str); 4] = [(2, "TENTATIVE"), (3, "ACCEPTED"), (4, "DECLINED"), (5, "NEEDS-ACTION")];
/// A meeting message's class, its calendar's `METHOD` and the answer it carries.
pub(crate) const MEETING_CLASSES: [(&str, &str, Option<&str>); 5] = [
    ("IPM.Schedule.Meeting.Request", "REQUEST", None),
    ("IPM.Schedule.Meeting.Canceled", "CANCEL", None),
    ("IPM.Schedule.Meeting.Resp.Pos", "REPLY", Some("ACCEPTED")),
    ("IPM.Schedule.Meeting.Resp.Neg", "REPLY", Some("DECLINED")),
    ("IPM.Schedule.Meeting.Resp.Tent", "REPLY", Some("TENTATIVE")),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The values the import and the export each had before they were kept here.
    #[test]
    fn the_values_are_outlooks() {
        assert_eq!((EPOCH_1601, EPOCH_MINUTES, NO_END), (11_644_473_600, 194_074_560, 0x5AE9_80DF));
        assert_eq!(PSETID_MEETING, GuidValue::new(0x6ED8_DA90, 0x450B, 0x101B, [0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05]));
        assert_eq!(
            [PSETID_ADDRESS, PSETID_APPOINTMENT, PSETID_TASK, PSETID_NOTE, PSETID_COMMON, PSETID_LOG].map(|g| g.data1()),
            [0x0006_2004, 0x0006_2002, 0x0006_2003, 0x0006_200E, 0x0006_2008, 0x0006_200A]
        );
        let phones: Vec<(u16, &str)> = PHONES.iter().flat_map(|(kind, ids)| ids.iter().map(move |id| (*id, *kind))).collect();
        assert_eq!(
            phones,
            [
                (0x3A08, "work"),
                (0x3A1B, "work"),
                (0x3A09, "home"),
                (0x3A2F, "home"),
                (0x3A1C, "cell"),
                (0x3A24, "fax"),
                (0x3A21, "pager"),
                (0x3A1F, "")
            ]
        );
    }

    /// The Exchange sign-in's FILETIME was a sum of its own, `(now + 11_644_473_600) *
    /// 10_000_000` in u64: the shared one gives the same for every time through the year 28000
    /// (it saturates at i64's end, near the year 30800); before 1601 it gives 0, and nothing
    /// overflows.
    #[test]
    fn filetime_is_the_sign_ins_old_sum() {
        for now in (0..=850_000_000_000u64).step_by(9_973_331).chain([1_763_164_800, u64::from(u32::MAX), 850_000_000_000]) {
            let old = (now + 11_644_473_600) * 10_000_000;
            assert_eq!(u64::try_from(filetime(i64::try_from(now).unwrap())), Ok(old), "{now}");
        }
        assert_eq!((filetime(-EPOCH_1601), filetime(i64::MIN), filetime(i64::MAX)), (0, 0, i64::MAX));
    }
}
