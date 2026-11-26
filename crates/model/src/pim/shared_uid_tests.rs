use super::*;

/// Duplicates get their start appended (a counter when the start repeats too); a lone
/// event and a recurring one in another calendar keep their UID.
#[test]
fn repeated_uids_get_their_start_appended() {
    let ev = |cal: &str, start: i64, rrule: Option<&str>| StoredEvent {
        calendar: cal.into(),
        event: CalEvent { uid: "series-1".into(), start, rrule: rrule.map(str::to_string), ..Default::default() },
        ..Default::default()
    };
    let mut list = vec![ev("cal", 100, None), ev("cal", 200, None), ev("cal", 100, None), ev("other", 100, Some("FREQ=WEEKLY"))];
    separate_shared_uids(&mut list);
    let uids: Vec<&str> = list.iter().map(|e| e.event.uid.as_str()).collect();
    assert_eq!(uids, ["series-1/100", "series-1/200", "series-1/100-2", "series-1"]);
    // Nothing to tell apart: unchanged.
    let mut one = vec![ev("cal", 100, None)];
    separate_shared_uids(&mut one);
    assert_eq!(one[0].event.uid, "series-1");
}
