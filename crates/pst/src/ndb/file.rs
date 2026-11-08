//! A data file opened to read: positioned reads and a small cache of B-tree pages.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::rc::Rc;

use super::header::{HEADER_READ, Header};
use super::*;
use crate::crypt::Crypt;
use crate::util::invalid;

/// What a data file is read through.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// B-tree pages kept, in each of the two generations (see [`Pages`]): the top levels of both
/// trees and the leaves in use. About 2 MiB at the most.
const PAGES_KEPT: usize = 2048;

/// The most a value (or a table's rows) may be when nothing sets a limit.
const NO_LIMIT: u64 = u64::MAX;

/// B-tree pages read, by where they are. When the young generation is full it becomes the old
/// one, and what was old goes: pages in use stay (a hit in the old generation moves up), the
/// memory held has a fixed bound, and nothing is scanned.
#[derive(Default)]
struct Pages {
    young: HashMap<u64, Rc<[u8]>>,
    old: HashMap<u64, Rc<[u8]>>,
}

impl Pages {
    fn get(&mut self, at: u64) -> Option<Rc<[u8]>> {
        if let Some(p) = self.young.get(&at) {
            return Some(p.clone());
        }
        let p = self.old.remove(&at)?;
        self.put(at, p.clone());
        Some(p)
    }

    fn put(&mut self, at: u64, page: Rc<[u8]>) {
        if self.young.len() >= PAGES_KEPT {
            self.old = std::mem::take(&mut self.young);
        }
        self.young.insert(at, page);
    }
}

/// An Outlook data file, read through a [`ReadSeek`].
pub struct PstFile {
    io: RefCell<Box<dyn ReadSeek>>,
    pub(crate) header: Header,
    max_value: Cell<u64>,
    pages: RefCell<Pages>,
}

impl PstFile {
    /// A data file read through `reader`, from its start.
    pub fn read_from(mut reader: Box<dyn ReadSeek>) -> io::Result<PstFile> {
        reader.seek(SeekFrom::Start(0))?;
        let mut head = Vec::with_capacity(HEADER_READ);
        (&mut reader).take(HEADER_READ as u64).read_to_end(&mut head)?;
        let header = Header::parse(&head)?;
        Ok(PstFile { io: RefCell::new(reader), header, max_value: Cell::new(NO_LIMIT), pages: RefCell::default() })
    }

    /// The data file at `path`, opened to read.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: impl AsRef<std::path::Path>) -> io::Result<PstFile> {
        PstFile::read_from(Box::new(std::fs::File::open(path)?))
    }

    /// Which of the two formats it is in.
    pub fn format(&self) -> Format {
        self.header.format
    }

    /// The most bytes a single value, or a table's rows, may be read as: anything declared
    /// bigger is an error before any of it is read.
    pub fn set_max_value_size(&self, bytes: u64) {
        self.max_value.set(bytes);
    }

    pub(crate) fn max_value(&self) -> u64 {
        self.max_value.get()
    }

    pub(crate) fn crypt(&self) -> Crypt {
        self.header.crypt
    }

    /// Exactly `buf.len()` bytes from `at`.
    pub(crate) fn read_at(&self, at: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut io = self.io.try_borrow_mut().map_err(|_| io::Error::other("the file is being read already"))?;
        io.seek(SeekFrom::Start(at))?;
        io.read_exact(buf)
    }

    /// The page at `at`, of type `ptype` and ID `bid`, checked. Pages are kept for the next
    /// time; `bid` 0 is any.
    pub(crate) fn page(&self, at: u64, ptype: u8, bid: u64) -> io::Result<Rc<[u8]>> {
        let mut pages = self.pages.try_borrow_mut().map_err(|_| io::Error::other("the page cache is in use"))?;
        if let Some(p) = pages.get(at) {
            check_page(&p, self.header.format, ptype, bid)?;
            return Ok(p);
        }
        let mut buf = vec![0u8; PAGE];
        self.read_at(at, &mut buf)?;
        check_page(&buf, self.header.format, ptype, bid)?;
        let page: Rc<[u8]> = buf.into();
        pages.put(at, page.clone());
        Ok(page)
    }
}

/// Whether a page's trailer says what it should.
fn check_page(b: &[u8], format: Format, ptype: u8, bid: u64) -> io::Result<()> {
    use crate::util::{u8_at, u32_at, u64_at};
    let data = format.page_data();
    let (found, again) = (u8_at(b, data)?, u8_at(b, data + 1)?);
    if found != ptype || again != ptype {
        return Err(invalid(format!("a page of type {found:#x} where {ptype:#x} should be")));
    }
    let (stored_crc, id) =
        if format.wide() { (u32_at(b, data + 4)?, u64_at(b, data + 8)?) } else { (u32_at(b, data + 8)?, u64::from(u32_at(b, data + 4)?)) };
    let body = b.get(..data).ok_or_else(crate::util::short)?;
    if !crate::crc::matches(body, stored_crc) {
        return Err(invalid("a page is damaged"));
    }
    if bid != 0 && id != bid {
        return Err(invalid("a page isn't the one named"));
    }
    Ok(())
}
