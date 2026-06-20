use alloc::{string::String, vec::Vec};
use core::str;

const NEWC_MAGIC: &[u8; 6] = b"070701";
const NEWC_HEADER_LEN: usize = 110;
const TRAILER: &str = "TRAILER!!!";

#[derive(Debug)]
pub enum InitramfsError {
    AddressOverflow,
    InvalidArchive,
    InvalidHex,
    InvalidName,
    NotFound,
    OutOfMemory,
}

pub struct Initramfs<'a> {
    archive: &'a [u8],
}

impl<'a> Initramfs<'a> {
    pub fn parse(archive: &'a [u8]) -> Result<Self, InitramfsError> {
        let initramfs = Self { archive };
        initramfs.validate()?;
        Ok(initramfs)
    }

    pub fn lookup(&self, path: &str) -> Result<&'a [u8], InitramfsError> {
        let needle = normalize_path(path);
        let mut cursor = 0;
        while cursor < self.archive.len() {
            let entry = parse_entry(self.archive, cursor)?;
            if entry.name == TRAILER {
                return Err(InitramfsError::NotFound);
            }
            if normalize_path(entry.name) == needle {
                return Ok(entry.data);
            }
            cursor = entry.next;
        }
        Err(InitramfsError::InvalidArchive)
    }

    fn validate(&self) -> Result<(), InitramfsError> {
        let mut cursor = 0;
        loop {
            let entry = parse_entry(self.archive, cursor)?;
            cursor = entry.next;
            if entry.name == TRAILER {
                return Ok(());
            }
            if cursor >= self.archive.len() {
                return Err(InitramfsError::InvalidArchive);
            }
        }
    }
}

pub fn build_single_file_newc(name: &str, data: &[u8]) -> Result<Vec<u8>, InitramfsError> {
    let mut archive = Vec::new();
    append_newc_entry(&mut archive, name, data)?;
    append_newc_entry(&mut archive, TRAILER, &[])?;
    Ok(archive)
}

struct Entry<'a> {
    name: &'a str,
    data: &'a [u8],
    next: usize,
}

fn parse_entry(archive: &[u8], cursor: usize) -> Result<Entry<'_>, InitramfsError> {
    let header_end = cursor
        .checked_add(NEWC_HEADER_LEN)
        .ok_or(InitramfsError::AddressOverflow)?;
    let header = archive
        .get(cursor..header_end)
        .ok_or(InitramfsError::InvalidArchive)?;
    if header.get(0..6) != Some(&NEWC_MAGIC[..]) {
        return Err(InitramfsError::InvalidArchive);
    }

    let file_size = parse_hex_field(header, 54)? as usize;
    let name_size = parse_hex_field(header, 94)? as usize;
    if name_size == 0 {
        return Err(InitramfsError::InvalidName);
    }

    let name_start = header_end;
    let name_end = name_start
        .checked_add(name_size)
        .ok_or(InitramfsError::AddressOverflow)?;
    let name_bytes = archive
        .get(name_start..name_end)
        .ok_or(InitramfsError::InvalidArchive)?;
    if name_bytes.last() != Some(&0) {
        return Err(InitramfsError::InvalidName);
    }
    let name =
        str::from_utf8(&name_bytes[..name_size - 1]).map_err(|_| InitramfsError::InvalidName)?;

    let data_start = align_up(name_end, 4).ok_or(InitramfsError::AddressOverflow)?;
    let data_end = data_start
        .checked_add(file_size)
        .ok_or(InitramfsError::AddressOverflow)?;
    let data = archive
        .get(data_start..data_end)
        .ok_or(InitramfsError::InvalidArchive)?;
    let next = align_up(data_end, 4).ok_or(InitramfsError::AddressOverflow)?;
    if next > archive.len() {
        return Err(InitramfsError::InvalidArchive);
    }

    Ok(Entry { name, data, next })
}

fn append_newc_entry(archive: &mut Vec<u8>, name: &str, data: &[u8]) -> Result<(), InitramfsError> {
    let name_size = name
        .len()
        .checked_add(1)
        .ok_or(InitramfsError::AddressOverflow)?;
    archive
        .try_reserve(
            NEWC_HEADER_LEN
                .checked_add(name_size)
                .and_then(|size| size.checked_add(data.len()))
                .and_then(|size| size.checked_add(8))
                .ok_or(InitramfsError::AddressOverflow)?,
        )
        .map_err(|_| InitramfsError::OutOfMemory)?;

    let mut header = String::from("070701");
    push_hex(&mut header, 1)?;
    push_hex(&mut header, 0o100755)?;
    push_hex(&mut header, 0)?;
    push_hex(&mut header, 0)?;
    push_hex(&mut header, 1)?;
    push_hex(&mut header, 0)?;
    push_hex(&mut header, data.len() as u32)?;
    push_hex(&mut header, 0)?;
    push_hex(&mut header, 0)?;
    push_hex(&mut header, 0)?;
    push_hex(&mut header, 0)?;
    push_hex(&mut header, name_size as u32)?;
    push_hex(&mut header, 0)?;
    if header.len() != NEWC_HEADER_LEN {
        return Err(InitramfsError::InvalidArchive);
    }

    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(name.as_bytes());
    archive.push(0);
    pad_to_4(archive);
    archive.extend_from_slice(data);
    pad_to_4(archive);
    Ok(())
}

fn push_hex(output: &mut String, value: u32) -> Result<(), InitramfsError> {
    use core::fmt::Write;

    write!(output, "{value:08x}").map_err(|_| InitramfsError::OutOfMemory)
}

fn parse_hex_field(header: &[u8], offset: usize) -> Result<u32, InitramfsError> {
    let field = header
        .get(offset..offset + 8)
        .ok_or(InitramfsError::InvalidArchive)?;
    let mut value = 0_u32;
    for byte in field {
        let digit = match *byte {
            b'0'..=b'9' => u32::from(*byte - b'0'),
            b'a'..=b'f' => u32::from(*byte - b'a' + 10),
            b'A'..=b'F' => u32::from(*byte - b'A' + 10),
            _ => return Err(InitramfsError::InvalidHex),
        };
        value = (value << 4) | digit;
    }
    Ok(value)
}

fn normalize_path(path: &str) -> &str {
    path.strip_prefix('/').unwrap_or(path)
}

fn pad_to_4(bytes: &mut Vec<u8>) {
    while bytes.len() & 3 != 0 {
        bytes.push(0);
    }
}

fn align_up(value: usize, align: usize) -> Option<usize> {
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}
