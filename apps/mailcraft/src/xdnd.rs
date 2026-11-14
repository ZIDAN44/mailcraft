//! Dragging a file out of the window on Linux and the BSDs under X11: MailCraft is the source of
//! an XDND drag (the freedesktop.org drag-and-drop protocol, version 5), offering the file as
//! `text/uri-list`. The drag runs on a connection of its own (x11rb, safe Rust): it follows the
//! pointer, talks to the window under it, and hands over the file's address when the button is
//! let go there. Under a Wayland session the window isn't an X window, so there is no drag.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, KeyButMask, PropMode, SELECTION_NOTIFY_EVENT,
    SelectionNotifyEvent, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

const VERSION: u32 = 5;
/// How often the pointer is looked at.
const TICK: Duration = Duration::from_millis(15);
/// The longest a drag lasts, and how long a drop target gets to take the file.
const MAX_DRAG: Duration = Duration::from_secs(120);
const MAX_FINISH: Duration = Duration::from_secs(5);
/// How long a target gets to answer a position before it's told the next one anyway.
const STATUS_WAIT: Duration = Duration::from_secs(1);

struct Atoms {
    aware: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    leave: Atom,
    drop: Atom,
    finished: Atom,
    selection: Atom,
    copy: Atom,
    uri_list: Atom,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A path as a `file://` address (RFC 8089), each byte that isn't plain percent-encoded: the
/// path's own bytes, so a name that isn't UTF-8 names the same file.
pub fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Drag `path` out of `own` (our window, which never takes the drop itself), until the button is
/// let go. `display`: the X display (`None`: `$DISPLAY`). `Ok(true)` when a window took it.
pub fn drag(display: Option<&str>, own: Window, path: &Path) -> Result<bool, String> {
    let (conn, screen_num) = x11rb::connect(display).map_err(err)?;
    let root = conn.setup().roots.get(screen_num).map(|s| s.root).ok_or("no screen")?;
    let atom = |name: &str| -> Result<Atom, String> { Ok(conn.intern_atom(false, name.as_bytes()).map_err(err)?.reply().map_err(err)?.atom) };
    let a = Atoms {
        aware: atom("XdndAware")?,
        enter: atom("XdndEnter")?,
        position: atom("XdndPosition")?,
        status: atom("XdndStatus")?,
        leave: atom("XdndLeave")?,
        drop: atom("XdndDrop")?,
        finished: atom("XdndFinished")?,
        selection: atom("XdndSelection")?,
        copy: atom("XdndActionCopy")?,
        uri_list: atom("text/uri-list")?,
    };
    // An unseen window to send from and to own the dragged data.
    let src = conn.generate_id().map_err(err)?;
    conn.create_window(0, src, root, -10, -10, 1, 1, 0, WindowClass::INPUT_ONLY, x11rb::COPY_FROM_PARENT, &CreateWindowAux::new()).map_err(err)?;
    conn.set_selection_owner(src, a.selection, x11rb::CURRENT_TIME).map_err(err)?;
    conn.flush().map_err(err)?;
    let data = format!("{}\r\n", file_uri(path)).into_bytes();
    let result = run(&conn, root, own, src, &a, &data);
    let _ = conn.destroy_window(src);
    let _ = conn.flush();
    result
}

fn send(conn: &impl Connection, to: Window, kind: Atom, data: [u32; 5]) -> Result<(), String> {
    let ev = ClientMessageEvent::new(32, to, kind, data);
    conn.send_event(false, to, EventMask::NO_EVENT, ev).map_err(err)?;
    Ok(())
}

/// The pointer: the top-level window under it, its place on the screen, and whether the button
/// is still held.
struct Pointer {
    child: Window,
    x: i16,
    y: i16,
    held: bool,
}

fn pointer(conn: &impl Connection, root: Window) -> Result<Pointer, String> {
    let p = conn.query_pointer(root).map_err(err)?.reply().map_err(err)?;
    Ok(Pointer { child: p.child, x: p.root_x, y: p.root_y, held: p.mask.contains(KeyButMask::BUTTON1) })
}

/// The XDND-aware window under the pointer, down from the top-level window `w` (`None` over
/// ours, or over none that takes drops).
fn aware_window(conn: &impl Connection, mut w: Window, own: Window, aware: Atom) -> Result<Option<Window>, String> {
    for _ in 0..16 {
        if w == x11rb::NONE || w == own {
            return Ok(None);
        }
        let prop = conn.get_property(false, w, aware, AtomEnum::ATOM, 0, 1).map_err(err)?.reply().map_err(err)?;
        if prop.value32().and_then(|mut v| v.next()).is_some_and(|v| v >= 3) {
            return Ok(Some(w));
        }
        w = conn.query_pointer(w).map_err(err)?.reply().map_err(err)?.child;
    }
    Ok(None)
}

/// What's done once the button is let go over a target.
#[derive(Debug, PartialEq)]
enum Release {
    /// Nothing yet: the target is still to be told the place, or to answer it.
    Wait,
    Drop,
    Leave,
}

/// What to do once the button is let go: the drop is made where the target was last told the
/// pointer is, so it's told the place the button was let go at first (`told_there`) and given
/// its time to answer (`answered`, or `waited` out), then dropped on if it takes drops
/// (`accepted`, as it last said).
fn on_release(told_there: bool, answered: bool, waited: bool, accepted: bool) -> Release {
    if !told_there || !(answered || waited) {
        Release::Wait
    } else if accepted {
        Release::Drop
    } else {
        Release::Leave
    }
}

fn run(conn: &impl Connection, root: Window, own: Window, src: Window, a: &Atoms, data: &[u8]) -> Result<bool, String> {
    let start = Instant::now();
    let mut target: Option<Window> = None;
    let mut accepted = false;
    let mut dropped_at: Option<Instant> = None;
    // Where the pointer was last looked at, and the window that takes drops there: looked for
    // again (a round trip for each window down from the top) only when the pointer moved.
    let mut seen: Option<(Window, i16, i16)> = None;
    let mut under: Option<Window> = None;
    // The place the target was last told of, when, and whether it answered.
    let mut told: Option<(i16, i16)> = None;
    let mut told_at = Instant::now();
    let mut answered = true;
    // Where the button was let go, once it has been: the pointer isn't followed after that.
    let mut released: Option<(i16, i16)> = None;
    loop {
        // What the target says, and its asks for the data.
        while let Some(ev) = conn.poll_for_event().map_err(err)? {
            match ev {
                Event::ClientMessage(m) if m.type_ == a.status => {
                    let d = m.data.as_data32();
                    if Some(d[0]) == target {
                        accepted = d[1] & 1 != 0;
                        answered = true;
                    }
                }
                Event::ClientMessage(m) if m.type_ == a.finished => return Ok(true),
                Event::SelectionRequest(r) if r.selection == a.selection => {
                    let ok = r.target == a.uri_list;
                    if ok {
                        conn.change_property8(PropMode::REPLACE, r.requestor, r.property, a.uri_list, data).map_err(err)?;
                    }
                    let reply = SelectionNotifyEvent {
                        response_type: SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: r.time,
                        requestor: r.requestor,
                        selection: r.selection,
                        target: r.target,
                        property: if ok { r.property } else { x11rb::NONE },
                    };
                    conn.send_event(false, r.requestor, EventMask::NO_EVENT, reply).map_err(err)?;
                }
                _ => {}
            }
        }
        conn.flush().map_err(err)?;
        if let Some(at) = dropped_at {
            // Dropped: waiting for the target to say it's done. One that never does isn't
            // known to have taken the file.
            if at.elapsed() > MAX_FINISH {
                return Ok(false);
            }
            std::thread::sleep(TICK);
            continue;
        }
        if start.elapsed() > MAX_DRAG {
            if let Some(t) = target {
                send(conn, t, a.leave, [src, 0, 0, 0, 0])?;
            }
            return Ok(false);
        }
        let place = match released {
            Some(at) => at,
            None => {
                let p = pointer(conn, root)?;
                if seen != Some((p.child, p.x, p.y)) {
                    under = aware_window(conn, p.child, own, a.aware)?;
                    seen = Some((p.child, p.x, p.y));
                }
                if under != target {
                    if let Some(t) = target {
                        send(conn, t, a.leave, [src, 0, 0, 0, 0])?;
                    }
                    if let Some(t) = under {
                        send(conn, t, a.enter, [src, VERSION << 24, a.uri_list, 0, 0])?;
                    }
                    target = under;
                    accepted = false;
                    (told, answered) = (None, true);
                }
                if !p.held {
                    released = Some((p.x, p.y));
                }
                (p.x, p.y)
            }
        };
        let waited = told_at.elapsed() > STATUS_WAIT;
        let Some(t) = target else {
            if released.is_some() {
                return Ok(false);
            }
            conn.flush().map_err(err)?;
            std::thread::sleep(TICK);
            continue;
        };
        if told != Some(place) && (answered || waited) {
            // Told where the pointer is when it moved, one place at a time as XDND asks: the
            // next once the target answered the last (or hasn't for a while). The place the
            // button is let go at is told too, before the drop: a target drops where it was
            // last told (one let go on just after a move had the file at the place before).
            let at = (u32::from(place.0.max(0).unsigned_abs()) << 16) | u32::from(place.1.max(0).unsigned_abs());
            send(conn, t, a.position, [src, 0, at, x11rb::CURRENT_TIME, a.copy])?;
            (told, told_at, answered) = (Some(place), Instant::now(), false);
        } else if released.is_some() {
            match on_release(told == Some(place), answered, waited, accepted) {
                Release::Wait => {}
                Release::Drop => {
                    send(conn, t, a.drop, [src, 0, x11rb::CURRENT_TIME, 0, 0])?;
                    dropped_at = Some(Instant::now());
                }
                Release::Leave => {
                    send(conn, t, a.leave, [src, 0, 0, 0, 0])?;
                    return Ok(false);
                }
            }
        }
        conn.flush().map_err(err)?;
        std::thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::{BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, MOTION_NOTIFY_EVENT};
    use x11rb::protocol::xtest::ConnectionExt as _;

    /// A file's address names its bytes: a name that isn't UTF-8 (found in audit, #243) became
    /// `%EF%BF%BD`, a file that isn't there.
    #[test]
    fn addresses_are_escaped() {
        assert_eq!(file_uri(Path::new("/tmp/mailcraft/Plan v2 (final).pdf")), "file:///tmp/mailcraft/Plan%20v2%20%28final%29.pdf");
        assert_eq!(file_uri(Path::new("/tmp/café")), "file:///tmp/caf%C3%A9");
        assert_eq!(file_uri(Path::new(std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9.eml"))), "file:///tmp/caf%E9.eml");
    }

    /// Once the button is let go, the target is told the place it was let go at before it's
    /// dropped on, and given its time to answer: the drop went out at once, at the place the
    /// target was told before.
    #[test]
    fn the_drop_waits_for_the_place_it_is_at() {
        // Let go where the target was last told, and it has answered: at once.
        assert_eq!(on_release(true, true, false, true), Release::Drop);
        assert_eq!(on_release(true, true, false, false), Release::Leave);
        // Let go somewhere it hasn't been told: told first, whatever it said of the place before.
        for (answered, waited) in [(true, false), (false, false), (false, true), (true, true)] {
            assert_eq!(on_release(false, answered, waited, true), Release::Wait, "{answered} {waited}");
        }
        // Told, not answered yet: waited for, then dropped on as it last said.
        assert_eq!(on_release(true, false, false, true), Release::Wait);
        assert_eq!(on_release(true, false, true, true), Release::Drop);
        assert_eq!(on_release(true, false, true, false), Release::Leave);
    }

    /// A whole drag on a real X server (Xvfb, when it's installed): a window that takes drops
    /// gets the file's address. While the pointer stands still over it, it's told the place
    /// once (found in audit, #243: a position every 15 ms, answered or not). Let go just after
    /// a move, it's told that place before the drop.
    #[test]
    fn a_file_is_dropped_on_a_window_that_takes_it() {
        let Some(xvfb) = ["/usr/bin/Xvfb", "/usr/local/bin/Xvfb"].iter().find(|p| Path::new(p).exists()) else { return };
        let display = format!(":{}", 70 + std::process::id() % 20);
        let Ok(mut server) = std::process::Command::new(xvfb).args([display.as_str(), "-screen", "0", "800x600x24", "-nolisten", "tcp"]).spawn()
        else {
            return;
        };
        let conn = (0..50).find_map(|_| {
            std::thread::sleep(Duration::from_millis(100));
            x11rb::connect(Some(&display)).ok()
        });
        let Some((conn, screen)) = conn else {
            let _ = server.kill();
            return;
        };
        let root = conn.setup().roots[screen].root;
        let atom = |n: &str| conn.intern_atom(false, n.as_bytes()).unwrap().reply().unwrap().atom;
        // Our window (left) and a drop target (right) that says it takes drops.
        let own = conn.generate_id().unwrap();
        conn.create_window(0, own, root, 0, 0, 300, 300, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new()).unwrap();
        let target = conn.generate_id().unwrap();
        conn.create_window(0, target, root, 400, 0, 300, 300, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new()).unwrap();
        conn.change_property32(PropMode::REPLACE, target, atom("XdndAware"), AtomEnum::ATOM, &[5]).unwrap();
        conn.map_window(own).unwrap();
        conn.map_window(target).unwrap();
        // The button goes down in our window.
        conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 100, 100, 0).unwrap();
        conn.xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0).unwrap();
        conn.flush().unwrap();
        let file = std::env::temp_dir().join("mailcraft drag.txt");
        let (d, f) = (display.clone(), file.clone());
        let source = std::thread::spawn(move || drag(Some(&d), own, &f));

        // The drop target's side of the protocol.
        let (enter, position, drop, uri, prop) =
            (atom("XdndEnter"), atom("XdndPosition"), atom("XdndDrop"), atom("text/uri-list"), atom("MAILCRAFT_DROP"));
        let (status, finished, selection) = (atom("XdndStatus"), atom("XdndFinished"), atom("XdndSelection"));
        std::thread::sleep(Duration::from_millis(200));
        conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 500, 100, 0).unwrap();
        conn.flush().unwrap();
        let mut got = None;
        let (mut positions, mut first_position) = (0, None::<Instant>);
        // The place the target was last told, as the drop came.
        let (mut last_place, mut dropped_at_place) = (0, None::<u32>);
        let mut released = false;
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end && got.is_none() {
            while let Some(ev) = conn.poll_for_event().unwrap() {
                match ev {
                    Event::ClientMessage(m) if m.type_ == enter => {
                        assert_eq!(m.data.as_data32()[2], uri, "offers a list of addresses");
                    }
                    Event::ClientMessage(m) if m.type_ == position => {
                        positions += 1;
                        first_position.get_or_insert_with(Instant::now);
                        last_place = m.data.as_data32()[2];
                        let from = m.data.as_data32()[0];
                        conn.send_event(
                            false,
                            from,
                            EventMask::NO_EVENT,
                            ClientMessageEvent::new(32, from, status, [target, 1, 0, 0, m.data.as_data32()[4]]),
                        )
                        .unwrap();
                    }
                    Event::ClientMessage(m) if m.type_ == drop => {
                        dropped_at_place = Some(last_place);
                        conn.convert_selection(target, selection, uri, prop, x11rb::CURRENT_TIME).unwrap();
                    }
                    Event::SelectionNotify(n) if n.property == prop => {
                        let v = conn.get_property(true, target, prop, AtomEnum::ANY, 0, 4096).unwrap().reply().unwrap();
                        got = Some(String::from_utf8(v.value).unwrap());
                        let from = conn.get_selection_owner(selection).unwrap().reply().unwrap().owner;
                        conn.send_event(
                            false,
                            from,
                            EventMask::NO_EVENT,
                            ClientMessageEvent::new(32, from, finished, [target, 1, atom("XdndActionCopy"), 0, 0]),
                        )
                        .unwrap();
                    }
                    _ => {}
                }
            }
            // Accepted, and the pointer held still over the target for half a second (some 30
            // of the drag's looks at it): told once. Then moved and let go at once, elsewhere
            // over it.
            if !released && first_position.is_some_and(|t| t.elapsed() > Duration::from_millis(500)) {
                assert_eq!(positions, 1, "a position for each look while the pointer stood still");
                conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 600, 200, 0).unwrap();
                conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0).unwrap();
                released = true;
            }
            conn.flush().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        let taken = source.join().unwrap();
        let _ = server.kill();
        assert_eq!(got.as_deref(), Some(format!("{}\r\n", file_uri(&file)).as_str()));
        assert_eq!(taken, Ok(true));
        assert_eq!(dropped_at_place, Some((600 << 16) | 200), "dropped where the button was let go");
    }
}
