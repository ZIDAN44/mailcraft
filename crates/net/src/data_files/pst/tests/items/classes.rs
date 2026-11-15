//! An item's class is told in one place, as each of the lists it was told by told it.

use super::super::super::item::{Class, EXCEPTION};
use super::super::super::lanes::{is_mail, is_pim};

/// The classes Outlook writes (and some that only start the same), in any case.
const CLASSES: [&str; 40] = [
    "",
    "IPM",
    "ipm.",
    "IPM.Note",
    "IPM.Note.SMIME.MultipartSigned",
    "ipm.note.rules.oofTemplate.Microsoft",
    "IPM.Notes",
    "REPORT.IPM.Note.NDR",
    "Report.IPM.Note.IPNRN",
    "report",
    "IPM.Post",
    "IPM.Post.Rss",
    "IPM.Document.Word.Document.12",
    "IPM.Outlook.Recall",
    "IPM.Recall.Report.Success",
    "IPM.Sharing",
    "IPM.Sharing.Calendar",
    "IPM.Schedule.Meeting.Request",
    "IPM.Schedule.Meeting.Canceled",
    "IPM.Schedule.Meeting.Resp.Pos",
    "IPM.Schedule.Inquiry",
    "ipm.schedule",
    "IPM.Appointment",
    "IPM.Appointment.Location",
    "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}",
    "IPM.OLE.CLASS",
    "IPM.Contact",
    "IPM.Contact.Custom",
    "IPM.Contactx",
    "IPM.DistList",
    "IPM.Task",
    "IPM.TaskRequest",
    "IPM.TaskRequest.Accept",
    "ipm.taskrequest.update",
    "IPM.StickyNote",
    "IPM.Activity",
    "IPM.InfoPathForm.Expense",
    "IPC.MS.REN.USERFIELDS",
    "ÏPM.Note",
    "IPM.Contäct",
];

fn starts(class: &str, list: &[&str]) -> bool {
    let class = class.to_ascii_lowercase();
    list.iter().any(|c| class.starts_with(c))
}

/// How the import told mail (`is_mail`), contacts and the calendar's (`is_pim`), the items whose
/// calendar properties are read (`item_of`), a contact's picture (`photo`), a hidden occurrence
/// (`hidden`) and where a contact or calendar item goes (`pim_of`), before.
#[test]
fn a_class_is_told_as_each_list_told_it() {
    for class in CLASSES {
        let was_mail = class.is_empty()
            || starts(
                class,
                &[
                    "ipm.note",
                    "ipm.schedule",
                    "report.",
                    "ipm.post",
                    "ipm.document",
                    "ipm.outlook.recall",
                    "ipm.recall",
                    "ipm.sharing",
                    "ipm.taskrequest",
                ],
            );
        let was_pim = !starts(class, &["ipm.taskrequest"])
            && starts(class, &["ipm.contact", "ipm.distlist", "ipm.appointment", "ipm.task", "ipm.stickynote", "ipm.activity"]);
        let read_pim = starts(
            class,
            &["ipm.contact", "ipm.distlist", "ipm.appointment", "ipm.schedule.meeting", "ipm.task", "ipm.stickynote", "ipm.activity", EXCEPTION],
        );
        let c = Class::of(class);
        assert_eq!((is_mail(class), is_pim(class), c.has_pim_properties()), (was_mail, was_pim, read_pim), "{class:?}");
        // A contact's picture, a hidden occurrence's classes.
        assert_eq!(c == Class::Contact, starts(class, &["ipm.contact"]), "{class:?}");
        assert_eq!(c == Class::Appointment, starts(class, &["ipm.appointment"]), "{class:?}");
        assert_eq!(c == Class::Meeting, starts(class, &["ipm.schedule.meeting"]), "{class:?}");
        assert_eq!(c == Class::Sharing, starts(class, &["ipm.sharing"]), "{class:?}");
        // Where a contact or calendar item goes: the first of these it started with.
        let went = ["ipm.activity", "ipm.distlist", "ipm.stickynote", "ipm.task", "ipm.contact"].into_iter().find(|s| starts(class, &[s]));
        let goes = match c {
            Class::Journal => Some("ipm.activity"),
            Class::ContactGroup => Some("ipm.distlist"),
            Class::StickyNote => Some("ipm.stickynote"),
            Class::Task | Class::TaskRequest => Some("ipm.task"),
            Class::Contact => Some("ipm.contact"),
            _ => None,
        };
        assert_eq!(goes, went, "{class:?}");
    }
}
