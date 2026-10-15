//! Dragging a file out of the window on Linux and the BSDs under X11: MailCraft is the source of
//! an XDND drag (the freedesktop.org drag-and-drop protocol, version 5), offering the file as
//! `text/uri-list`. The drag runs on a connection of its own (x11rb, safe Rust): it follows the
//! pointer, talks to the window under it, and hands over the file's address when the button is
//! let go there. Under a Wayland session the window isn't an X window, so there is no drag.

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

/// A path as a `file://` address (RFC 8089), each byte that isn't plain percent-encoded.
pub fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
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

/// The XDND-aware window under the pointer (not ours), and the pointer's place and buttons.
fn under_pointer(conn: &impl Connection, root: Window, own: Window, aware: Atom) -> Result<(Option<Window>, i16, i16, bool), String> {
    let p = conn.query_pointer(root).map_err(err)?.reply().map_err(err)?;
    let held = p.mask.contains(KeyButMask::BUTTON1);
    let (x, y) = (p.root_x, p.root_y);
    let mut w = p.child;
    // Down from the top-level window to the one that says it takes drops.
    for _ in 0..16 {
        if w == x11rb::NONE {
            return Ok((None, x, y, held));
        }
        if w == own {
            return Ok((None, x, y, held));
        }
        let prop = conn.get_property(false, w, aware, AtomEnum::ATOM, 0, 1).map_err(err)?.reply().map_err(err)?;
        if prop.value32().and_then(|mut v| v.next()).is_some_and(|v| v >= 3) {
            return Ok((Some(w), x, y, held));
        }
        let q = conn.query_pointer(w).map_err(err)?.reply().map_err(err)?;
        w = q.child;
    }
    Ok((None, x, y, held))
}

fn run(conn: &impl Connection, root: Window, own: Window, src: Window, a: &Atoms, data: &[u8]) -> Result<bool, String> {
    let start = Instant::now();
    let mut target: Option<Window> = None;
    let mut accepted = false;
    let mut dropped_at: Option<Instant> = None;
    loop {
        // What the target says, and its asks for the data.
        while let Some(ev) = conn.poll_for_event().map_err(err)? {
            match ev {
                Event::ClientMessage(m) if m.type_ == a.status => {
                    let d = m.data.as_data32();
                    if Some(d[0]) == target {
                        accepted = d[1] & 1 != 0;
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
            // Dropped: waiting for the target to say it's done.
            if at.elapsed() > MAX_FINISH {
                return Ok(accepted);
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
        let (under, x, y, held) = under_pointer(conn, root, own, a.aware)?;
        if under != target {
            if let Some(t) = target {
                send(conn, t, a.leave, [src, 0, 0, 0, 0])?;
            }
            if let Some(t) = under {
                send(conn, t, a.enter, [src, VERSION << 24, a.uri_list, 0, 0])?;
            }
            target = under;
            accepted = false;
        }
        if !held {
            match target {
                Some(t) if accepted => {
                    send(conn, t, a.drop, [src, 0, x11rb::CURRENT_TIME, 0, 0])?;
                    dropped_at = Some(Instant::now());
                }
                Some(t) => {
                    send(conn, t, a.leave, [src, 0, 0, 0, 0])?;
                    return Ok(false);
                }
                None => return Ok(false),
            }
        } else if let Some(t) = target {
            let at = (u32::from(x.max(0).unsigned_abs()) << 16) | u32::from(y.max(0).unsigned_abs());
            send(conn, t, a.position, [src, 0, at, x11rb::CURRENT_TIME, a.copy])?;
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

    #[test]
    fn addresses_are_escaped() {
        assert_eq!(file_uri(Path::new("/tmp/mailcraft/Plan v2 (final).pdf")), "file:///tmp/mailcraft/Plan%20v2%20%28final%29.pdf");
        assert_eq!(file_uri(Path::new("/tmp/café")), "file:///tmp/caf%C3%A9");
    }

    /// A whole drag on a real X server (Xvfb, when it's installed): a window that takes drops
    /// gets the file's address.
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
        let mut released = false;
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end && got.is_none() {
            while let Some(ev) = conn.poll_for_event().unwrap() {
                match ev {
                    Event::ClientMessage(m) if m.type_ == enter => {
                        assert_eq!(m.data.as_data32()[2], uri, "offers a list of addresses");
                    }
                    Event::ClientMessage(m) if m.type_ == position => {
                        let from = m.data.as_data32()[0];
                        conn.send_event(
                            false,
                            from,
                            EventMask::NO_EVENT,
                            ClientMessageEvent::new(32, from, status, [target, 1, 0, 0, m.data.as_data32()[4]]),
                        )
                        .unwrap();
                        if !released {
                            // Accepted: let go over the target.
                            conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0).unwrap();
                            released = true;
                        }
                    }
                    Event::ClientMessage(m) if m.type_ == drop => {
                        conn.convert_selection(target, selection, uri, prop, x11rb::CURRENT_TIME).unwrap();
                        let _ = m;
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
            conn.flush().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        let taken = source.join().unwrap();
        let _ = server.kill();
        assert_eq!(got.as_deref(), Some(format!("{}\r\n", file_uri(&file)).as_str()));
        assert_eq!(taken, Ok(true));
    }
}
