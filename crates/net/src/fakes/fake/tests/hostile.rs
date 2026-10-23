//! A hostile IMAP server can't make one command use unbounded memory or time.

use super::*;
use crate::imap::Limits;

/// A `NOOP` answered by `flood`, over and over, each read of the connection taking `trickle`.
fn noop_against(flood: Vec<u8>, limits: Limits, trickle: Duration) -> std::result::Result<usize, crate::NetError> {
    let st = State::new();
    let mut c = login(&st);
    c.conn_mut().set_limits(limits);
    {
        let mut s = st.lock().unwrap();
        s.flood = Some(flood);
        s.trickle = trickle;
    }
    c.noop().map(|v| v.len())
}

#[test]
fn endless_untagged_responses_drop_the_connection() {
    let limits = Limits { responses: 5_000, ..Limits::default() };
    let r = noop_against(b"* 1 EXISTS\r\n".to_vec(), limits, Duration::ZERO);
    assert!(matches!(&r, Err(e @ crate::NetError::Protocol(_)) if e.is_fatal()), "{r:?}");
}

#[test]
fn many_big_literals_drop_the_connection() {
    // Each literal is fine on its own (well under MAX_LITERAL); together they pass the cap.
    let body = vec![b'x'; 256 << 10];
    let mut item = format!("* 1 FETCH (UID 1 BODY[] {{{}}}\r\n", body.len()).into_bytes();
    item.extend_from_slice(&body);
    item.extend_from_slice(b")\r\n");
    let limits = Limits { bytes: 4 << 20, ..Limits::default() };
    let r = noop_against(item, limits, Duration::ZERO);
    assert!(matches!(&r, Err(crate::NetError::Protocol(_))), "{r:?}");
}

#[test]
fn a_trickling_server_runs_out_of_time() {
    let limits = Limits { time: Duration::from_millis(300), ..Limits::default() };
    let start = Instant::now();
    let r = noop_against(b"* OK still here\r\n".to_vec(), limits, Duration::from_millis(2));
    assert!(matches!(&r, Err(crate::NetError::Protocol(_))), "{r:?}");
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn ordinary_commands_stay_within_the_limits() {
    let st = State::new();
    for i in 0..300 {
        st.lock().unwrap().add("INBOX", &format!("m{i}"), &[]);
    }
    let mut c = login(&st);
    c.conn_mut().set_limits(Limits { responses: 1_000, bytes: 1 << 20, time: Duration::from_secs(60) });
    c.select("INBOX", false).unwrap();
    assert_eq!(c.uid_fetch("1:*", "UID FLAGS BODY.PEEK[]", "").unwrap().len(), 300);
}
