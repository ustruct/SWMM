use std::borrow::Cow;
use std::convert::TryInto;

use camino::Utf8Path;
use chrono::{NaiveDate, NaiveDateTime};
use codepage_437::*;
use memchr::memmem;

use crate::arch::usize;
use crate::read::{CompressionMethod, FileMetadata};
use crate::result::*;

const EOCDR_MAGIC: [u8; 4] = [b'P', b'K', 5, 6];

const ZIP64_EOCDR_MAGIC: [u8; 4] = [b'P', b'K', 6, 6];

const ZIP64_EOCDR_LOCATOR_MAGIC: [u8; 4] = [b'P', b'K', 6, 7];

const CENTRAL_DIRECTORY_MAGIC: [u8; 4] = [b'P', b'K', 1, 2];

const LOCAL_FILE_HEADER_MAGIC: [u8; 4] = [b'P', b'K', 3, 4];

impl CompressionMethod {
    fn from_u16(u: u16) -> Self {
        match u {
            0 => CompressionMethod::None,
            8 => CompressionMethod::Deflate,

            v => CompressionMethod::Unsupported(v),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum System {
    Dos,
    Unix,
    Other(u8),
}

#[allow(dead_code)]
impl System {
    fn from_source_version(source_version: u16) -> Self {

        match source_version >> 8 {
            0 => System::Dos,
            3 => System::Unix,
            o => System::Other(o as u8),
        }
    }
}

fn read_u64(input: &mut &[u8]) -> u64 {
    let (int_bytes, rest) = input.split_at(std::mem::size_of::<u64>());
    *input = rest;
    u64::from_le_bytes(int_bytes.try_into().expect("less than eight bytes for u64"))
}

fn read_u32(input: &mut &[u8]) -> u32 {
    let (int_bytes, rest) = input.split_at(std::mem::size_of::<u32>());
    *input = rest;
    u32::from_le_bytes(int_bytes.try_into().expect("less than four bytes for u32"))
}

fn read_u16(input: &mut &[u8]) -> u16 {
    let (int_bytes, rest) = input.split_at(std::mem::size_of::<u16>());
    *input = rest;
    u16::from_le_bytes(int_bytes.try_into().expect("less than two bytes for u16"))
}

#[derive(Debug)]
pub struct EndOfCentralDirectory<'a> {
    pub disk_number: u16,
    pub disk_with_central_directory: u16,
    pub entries_on_this_disk: u16,
    pub entries: u16,
    pub central_directory_size: u32,
    pub central_directory_offset: u32,
    pub file_comment: &'a [u8],
}

impl<'a> EndOfCentralDirectory<'a> {
    pub fn parse(mut eocdr: &'a [u8]) -> ZipResult<Self> {

        assert_eq!(eocdr[..4], EOCDR_MAGIC);
        eocdr = &eocdr[4..];
        let disk_number = read_u16(&mut eocdr);
        let disk_with_central_directory = read_u16(&mut eocdr);
        let entries_on_this_disk = read_u16(&mut eocdr);
        let entries = read_u16(&mut eocdr);
        let central_directory_size = read_u32(&mut eocdr);
        let central_directory_offset = read_u32(&mut eocdr);
        let comment_length = read_u16(&mut eocdr);
        let file_comment = &eocdr[..usize(comment_length)?];

        Ok(Self {
            disk_number,
            disk_with_central_directory,
            entries_on_this_disk,
            entries,
            central_directory_size,
            central_directory_offset,
            file_comment,
        })
    }
}

pub fn find_eocdr(mapping: &[u8]) -> ZipResult<usize> {
    memmem::rfind(mapping, &EOCDR_MAGIC).ok_or(ZipError::InvalidArchive(
        "Couldn't find End Of Central Directory Record",
    ))
}

#[derive(Debug)]
pub struct Zip64EndOfCentralDirectoryLocator {
    pub disk_with_central_directory: u32,
    pub zip64_eocdr_offset: u64,
    pub disks: u32,
}

impl Zip64EndOfCentralDirectoryLocator {
    pub fn parse(mut mapping: &[u8]) -> Option<Self> {

        if mapping[..4] != ZIP64_EOCDR_LOCATOR_MAGIC {
            return None;
        }
        mapping = &mapping[4..];
        let disk_with_central_directory = read_u32(&mut mapping);
        let zip64_eocdr_offset = read_u64(&mut mapping);
        let disks = read_u32(&mut mapping);

        Some(Self {
            disk_with_central_directory,
            zip64_eocdr_offset,
            disks,
        })
    }

    pub fn size_in_file() -> usize {
        20
    }
}

#[derive(Debug)]
pub struct Zip64EndOfCentralDirectory<'a> {
    pub source_version: u16,
    pub minimum_extract_version: u16,
    pub disk_number: u32,
    pub disk_with_central_directory: u32,
    pub entries_on_this_disk: u64,
    pub entries: u64,
    pub central_directory_size: u64,
    pub central_directory_offset: u64,
    pub extensible_data: &'a [u8],
}

impl<'a> Zip64EndOfCentralDirectory<'a> {
    pub fn parse(mut eocdr: &'a [u8]) -> ZipResult<Self> {

        assert_eq!(eocdr[..4], ZIP64_EOCDR_MAGIC);
        eocdr = &eocdr[4..];
        let eocdr_size = read_u64(&mut eocdr);
        let source_version = read_u16(&mut eocdr);
        let minimum_extract_version = read_u16(&mut eocdr);
        let disk_number = read_u32(&mut eocdr);
        let disk_with_central_directory = read_u32(&mut eocdr);
        let entries_on_this_disk = read_u64(&mut eocdr);
        let entries = read_u64(&mut eocdr);
        let central_directory_size = read_u64(&mut eocdr);
        let central_directory_offset = read_u64(&mut eocdr);

        let eocdr_size = usize(eocdr_size)?;
        if (eocdr_size + 12) < Self::fixed_size_in_file() {
            return Err(ZipError::InvalidArchive(
                "Invalid extensible data length in Zip64 End Of Central Directory Record",
            ));
        }

        let extensible_data_length = eocdr_size + 12 - Self::fixed_size_in_file();
        if eocdr.len() != extensible_data_length {
            return Err(ZipError::InvalidArchive(
                "Invalid extensible data length in Zip64 End Of Central Directory Record",
            ));
        }
        let extensible_data = eocdr;

        Ok(Self {
            source_version,
            minimum_extract_version,
            disk_number,
            disk_with_central_directory,
            entries,
            entries_on_this_disk,
            central_directory_size,
            central_directory_offset,
            extensible_data,
        })
    }

    fn fixed_size_in_file() -> usize {
        56
    }
}

pub fn find_zip64_eocdr(mapping: &[u8]) -> ZipResult<usize> {
    memmem::find(mapping, &ZIP64_EOCDR_MAGIC).ok_or(ZipError::InvalidArchive(
        "Couldn't find zip64 End Of Central Directory Record",
    ))
}

#[derive(Debug)]
pub struct CentralDirectoryEntry<'a> {
    pub source_version: u16,
    pub minimum_extract_version: u16,
    pub flags: u16,
    pub compression_method: u16,
    pub last_modified_time: u16,
    pub last_modified_date: u16,
    pub crc32: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub disk_number: u16,
    pub internal_file_attributes: u16,
    pub external_file_attributes: u32,
    pub header_offset: u32,
    pub path: &'a [u8],
    pub extra_field: &'a [u8],
    pub file_comment: &'a [u8],
}

impl<'a> CentralDirectoryEntry<'a> {
    pub fn parse_and_consume(entry: &mut &'a [u8]) -> ZipResult<Self> {

        if entry[..4] != CENTRAL_DIRECTORY_MAGIC {
            return Err(ZipError::InvalidArchive("Invalid central directory entry"));
        }
        *entry = &entry[4..];
        let source_version = read_u16(entry);
        let minimum_extract_version = read_u16(entry);
        let flags = read_u16(entry);
        let compression_method = read_u16(entry);
        let last_modified_time = read_u16(entry);
        let last_modified_date = read_u16(entry);
        let crc32 = read_u32(entry);
        let compressed_size = read_u32(entry);
        let uncompressed_size = read_u32(entry);
        let path_length = usize(read_u16(entry))?;
        let extra_field_length = usize(read_u16(entry))?;
        let file_comment_length = usize(read_u16(entry))?;
        let disk_number = read_u16(entry);
        let internal_file_attributes = read_u16(entry);
        let external_file_attributes = read_u32(entry);
        let header_offset = read_u32(entry);
        let (path, remaining) = entry.split_at(path_length);
        let (extra_field, remaining) = remaining.split_at(extra_field_length);
        let (file_comment, remaining) = remaining.split_at(file_comment_length);
        *entry = remaining;

        Ok(Self {
            source_version,
            minimum_extract_version,
            flags,
            compression_method,
            last_modified_time,
            last_modified_date,
            crc32,
            compressed_size,
            uncompressed_size,
            disk_number,
            internal_file_attributes,
            external_file_attributes,
            header_offset,
            path,
            extra_field,
            file_comment,
        })
    }
}

fn is_utf8(flags: u16) -> bool {

    flags & (1 << 11) != 0
}

fn is_encrypted(flags: u16) -> bool {

    flags & 1 != 0
}

impl<'a> FileMetadata<'a> {

    pub(crate) fn from_cde(cde: &CentralDirectoryEntry<'a>) -> ZipResult<Self> {
        let is_utf8 = is_utf8(cde.flags);

        let path: Cow<Utf8Path> = if is_utf8 {
            let utf8 = std::str::from_utf8(cde.path).map_err(ZipError::Encoding)?;
            Cow::Borrowed(Utf8Path::new(utf8))
        } else {
            let str_cow: Cow<str> = Cow::borrow_from_cp437(cde.path, &CP437_CONTROL);

            match str_cow {
                Cow::Borrowed(s) => Cow::Borrowed(Utf8Path::new(s)),
                Cow::Owned(s) => Cow::Owned(s.into()),
            }
        };

        if cde.disk_number != 0 {
            return Err(ZipError::UnsupportedArchive(format!(
                "No support for multi-disk archives: file {path} claims to be on disk {}",
                cde.disk_number,
            )));
        }

        let encrypted = is_encrypted(cde.flags);

        let compression_method = CompressionMethod::from_u16(cde.compression_method);

        let unix_mode = match System::from_source_version(cde.source_version) {

            System::Unix => Some((cde.external_file_attributes >> 16) as u16),
            _ => None,
        };

        let mut metadata = Self {
            size: usize(cde.uncompressed_size)?,
            compressed_size: usize(cde.compressed_size)?,
            compression_method,
            crc32: cde.crc32,
            encrypted,
            path,
            last_modified: parse_msdos(cde.last_modified_time, cde.last_modified_date),
            unix_mode,
            header_offset: usize(cde.header_offset)?,
        };

        parse_extra_field(&mut metadata, cde.extra_field, false)?;

        Ok(metadata)
    }

    pub(crate) fn from_local_header(
        local: &LocalFileHeader<'a>,
        cde_header: &Self,
    ) -> ZipResult<Self> {
        let is_utf8 = is_utf8(local.flags);

        let path: Cow<Utf8Path> = if is_utf8 {
            let utf8 = std::str::from_utf8(local.path).map_err(ZipError::Encoding)?;
            Cow::Borrowed(Utf8Path::new(utf8))
        } else {
            let str_cow: Cow<str> = Cow::borrow_from_cp437(local.path, &CP437_CONTROL);

            match str_cow {
                Cow::Borrowed(s) => Cow::Borrowed(Utf8Path::new(s)),
                Cow::Owned(s) => Cow::Owned(s.into()),
            }
        };

        let encrypted = is_encrypted(local.flags);

        let compression_method = CompressionMethod::from_u16(local.compression_method);

        let mut metadata = Self {
            size: usize(local.uncompressed_size)?,
            compressed_size: usize(local.compressed_size)?,
            compression_method,
            crc32: local.crc32,
            encrypted,
            path,
            last_modified: parse_msdos(local.last_modified_time, local.last_modified_date),
            ..*cde_header
        };

        parse_extra_field(&mut metadata, local.extra_field, true)?;

        Ok(metadata)
    }
}

fn parse_msdos(time: u16, date: u16) -> NaiveDateTime {
    let seconds = (0b0000_0000_0001_1111 & time) as u32 * 2;
    let minutes = (0b0000_0111_1110_0000 & time) as u32 >> 5;
    let hours = (0b1111_1000_0000_0000 & time) as u32 >> 11;

    let days = (0b0000_0000_0001_1111 & date) as u32;
    let months = (0b0000_0001_1110_0000 & date) as u32 >> 5;

    let years = ((0b1111_1110_0000_0000 & date) >> 9) as i32 + 1980;

    NaiveDate::from_ymd(years, months, days).and_hms(hours, minutes, seconds)
}

fn parse_extra_field(
    metadata: &mut FileMetadata,
    mut extra_field: &[u8],
    is_local: bool,
) -> ZipResult<()> {

    while extra_field.len() >= 4 {
        let kind = read_u16(&mut extra_field);
        let field_len = (read_u16(&mut extra_field) as usize).min(extra_field.len());
        let (mut data, rest) = extra_field.split_at(field_len);
        extra_field = rest;

        if kind == 0x0001 {
            for value in [
                &mut metadata.size,
                &mut metadata.compressed_size,
                &mut metadata.header_offset,
            ] {
                if *value == u32::MAX as usize {
                    if data.len() < 8 {
                        return Err(ZipError::InvalidArchive("Zip64 extra field is too short"));
                    }
                    *value = usize(read_u64(&mut data))?;
                }
            }
            if !data.is_empty() && !is_local {
                return Err(ZipError::InvalidArchive(
                    "Extra data field contains disk number",
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct LocalFileHeader<'a> {
    pub minimum_extract_version: u16,
    pub flags: u16,
    pub compression_method: u16,
    pub last_modified_time: u16,
    pub last_modified_date: u16,
    pub crc32: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub path: &'a [u8],
    pub extra_field: &'a [u8],
}

impl<'a> LocalFileHeader<'a> {
    pub fn parse_and_consume(header: &mut &'a [u8]) -> ZipResult<Self> {

        assert_eq!(header[..4], LOCAL_FILE_HEADER_MAGIC);
        *header = &header[4..];
        let minimum_extract_version = read_u16(header);
        let flags = read_u16(header);
        let compression_method = read_u16(header);
        let last_modified_time = read_u16(header);
        let last_modified_date = read_u16(header);
        let crc32 = read_u32(header);
        let compressed_size = read_u32(header);
        let uncompressed_size = read_u32(header);
        let path_length = usize(read_u16(header))?;
        let extra_field_length = usize(read_u16(header))?;
        let (path, remaining) = header.split_at(path_length);
        let (extra_field, remaining) = remaining.split_at(extra_field_length);
        *header = remaining;

        Ok(Self {
            minimum_extract_version,
            flags,
            compression_method,
            last_modified_time,
            last_modified_date,
            crc32,
            compressed_size,
            uncompressed_size,
            path,
            extra_field,
        })
    }
}
