use super::*;

#[test]
fn secrets_never_print() {
    let j = Job::Start { account: AccountConfig::default(), password: Some(Secret("hunter2".into())) };
    assert!(!format!("{j:?}").contains("hunter2"));
    let e = Event::Decrypted { key: MessageKey::default(), content: Some(Private(b"the plan".to_vec())), authenticated: true, error: None };
    assert!(!format!("{e:?}").contains("the plan"), "what decrypted mail says never prints");
    let j = Job::Pgp {
        account: AccountConfig::default(),
        work: PgpWork::Import { file: Private(b"-----BEGIN PGP PRIVATE KEY BLOCK-----".to_vec()), passphrase: Secret("hunter2".into()) },
    };
    let shown = format!("{j:?}");
    assert!(!shown.contains("hunter2") && !shown.contains("45, 45"), "neither the passphrase nor the key file: {shown}");
}

#[test]
fn jobs_serialize_tagged() {
    let j = Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() };
    let v = serde_json::to_value(&j).unwrap();
    assert_eq!(v["job"], "syncFolder");
    assert_eq!(serde_json::from_value::<Job>(v).unwrap(), j);
    assert_eq!(j.account(), "a1");
    let j = Job::Index { account: "a1".into(), folders: vec!["INBOX".into()] };
    assert_eq!((serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j.account()), (j.clone(), "a1"));
    assert_eq!(Job::StopIndex { account: "a1".into() }.account(), "a1");
    let e = Event::Indexed { account: "a1".into(), work: Work { done: 1, total: 2, ..Work::default() }, generation: 3, failed: 0, finished: false };
    assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
}

#[test]
fn import_events_read_what_earlier_versions_wrote() {
    // As an earlier version sent them: no report, no `resume`.
    let old = r#"{"event":"imported","account":"a1","messages":3,"skipped":1,"done":true,"error":null}"#;
    let e: Event = serde_json::from_str(old).unwrap();
    assert_eq!(e, Event::Imported { account: "a1".into(), messages: 3, skipped: 1, done: true, error: None, report: None });
    assert_eq!(serde_json::to_value(&e).unwrap(), serde_json::from_str::<serde_json::Value>(old).unwrap(), "written as it was");
    let j: Job = serde_json::from_str(r#"{"job":"importPst","account":"a1","path":"a.pst"}"#).unwrap();
    assert_eq!(j, Job::ImportPst { account: "a1".into(), path: "a.pst".into(), resume: false });

    // And the new ones round trip.
    let mut report = ImportReport { messages: 3, runs: 1, ..Default::default() };
    report.skip(SkipReason::AttachmentTooBig, 2);
    let e = Event::Imported { account: "a1".into(), messages: 3, skipped: 1, done: true, error: None, report: Some(Box::new(report)) };
    assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    let progress = ImportProgress { phase: ImportPhase::Importing, work: Work { done: 1, total: 9, ..Default::default() }, ..Default::default() };
    let e = Event::ImportProgress { account: "a1".into(), progress };
    assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    let folders = vec![PlannedFolder { path: "Projects/2024".into(), name: "2024".into(), items: 7 }];
    let e = Event::ImportPlanned { account: "a1".into(), folders };
    assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    let j = Job::StopImport { account: "a1".into(), how: StopHow::Discard };
    assert_eq!((serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j.account()), (j.clone(), "a1"));
    assert_eq!(Job::InspectDataFile { path: "a.pst".into() }.account(), "");
    let e = Event::ImportKept { account: "a1".into(), error: Some("full".into()) };
    assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    let j = Job::KeepImport { account: "a1".into() };
    assert_eq!((serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j.account()), (j.clone(), "a1"));
}

#[test]
fn compaction_jobs_and_events_round_trip() {
    for j in [
        Job::DataFileHealth { account: "a1".into() },
        Job::CompactDataFile { account: "a1".into(), mode: CompactMode::Manual },
        Job::CancelCompact { account: "a1".into() },
    ] {
        assert_eq!((serde_json::from_str::<Job>(&serde_json::to_string(&j).unwrap()).unwrap(), j.account()), (j.clone(), "a1"));
    }
    let health = DataFileHealth { file_bytes: 9, raw_fraction: 0.25, ..DataFileHealth::default() };
    let report = CompactReport { mode: CompactMode::Close, before: 9, after: 4, compacted: true, ..CompactReport::default() };
    for e in [
        Event::DataFileHealth { account: "a1".into(), health: Some(Box::new(health)), error: None },
        Event::Compacting { account: "a1".into(), phase: CompactPhase::Exclusive, work: Work::default(), exclusive_s: Some(12) },
        Event::Compacted { account: "a1".into(), report: Box::new(report) },
    ] {
        assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    }
}
