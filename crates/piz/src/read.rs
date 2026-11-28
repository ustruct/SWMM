use std::borrow::Cow;
use std::collections::{btree_map, BTreeMap};
use std::io;

use camino::{Utf8Component, Utf8Path};
use chrono::NaiveDateTime;
use flate2::read::DeflateDecoder;
use log::*;

use crate::arch::usize;
use crate::crc_reader::Crc32Reader;
use crate::result::*;
use crate::spec;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CompressionMethod {

    None,

    Deflate,

    Unsupported(u16),
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileMetadata<'a> {

    pub size: usize,

    pub compressed_size: usize,

    pub compression_method: CompressionMethod,

    pub crc32: u32,

    pub encrypted: bool,

    pub path: Cow<'a, Utf8Path>,

    pub last_modified: NaiveDateTime,

    pub unix_mode: Option<u16>,

    pub(crate) header_offset: usize,
}

impl<'a> FileMetadata<'a> {

    pub fn is_dir(&self) -> bool {

        self.size == 0 && self.path.as_str().ends_with('/')
    }

    pub fn is_file(&self) -> bool {
        !self.is_dir()
    }

    pub fn into_owned(self) -> FileMetadata<'static> {
        FileMetadata {
            path: Cow::Owned(self.path.into_owned()),
            ..self
        }
    }
}

pub struct ZipArchive<'a> {

    mapping: &'a [u8],

    entries: Vec<FileMetadata<'a>>,
}

impl<'a> ZipArchive<'a> {

    pub fn new(mapping: &'a [u8]) -> ZipResult<Self> {
        let (new_archive, archive_offset) = Self::with_prepended_data(mapping)?;
        if archive_offset != 0 {
            return Err(ZipError::PrependedWithUnknownBytes(archive_offset));
        }
        Ok(new_archive)
    }

    pub fn with_prepended_data(mut mapping: &'a [u8]) -> ZipResult<(Self, usize)> {
        let eocdr_posit = spec::find_eocdr(mapping)?;
        let eocdr = spec::EndOfCentralDirectory::parse(&mapping[eocdr_posit..])?;
        trace!("{:?}", eocdr);

        if eocdr.disk_number != eocdr.disk_with_central_directory {
            return Err(ZipError::UnsupportedArchive(format!(
                "No support for multi-disk archives: disk ({}) != disk with central directory ({})",
                eocdr.disk_number, eocdr.disk_with_central_directory
            )));
        }
        if eocdr.entries != eocdr.entries_on_this_disk {
            return Err(ZipError::UnsupportedArchive(format!(
                "No support for multi-disk archives: entries ({}) != entries this disk ({})",
                eocdr.entries, eocdr.entries_on_this_disk
            )));
        }

        let nominal_central_directory_offset: usize;
        let entry_count: u64;

        let archive_offset;

        let zip64_eocdr_locator_posit = eocdr_posit
            .checked_sub(spec::Zip64EndOfCentralDirectoryLocator::size_in_file())
            .ok_or(ZipError::InvalidArchive(
                "Too small for anything but End Of Central Directory Record",
            ))?;
        if let Some(zip64_eocdr_locator) =
            spec::Zip64EndOfCentralDirectoryLocator::parse(&mapping[zip64_eocdr_locator_posit..])
        {
            trace!("{:?}", zip64_eocdr_locator);

            if eocdr.disk_number as u32 != zip64_eocdr_locator.disk_with_central_directory {
                return Err(ZipError::UnsupportedArchive(format!(
                    "No support for multi-disk archives: disk ({}) != disk with zip64 central directory ({})",
                    eocdr.disk_number, zip64_eocdr_locator.disk_with_central_directory
                )));
            }
            if zip64_eocdr_locator.disks != 1 {
                return Err(ZipError::UnsupportedArchive(format!(
                    "No support for multi-disk archives: Zip64 EOCDR locator reports {} disks",
                    zip64_eocdr_locator.disks
                )));
            }

            let zip64_eocdr_search_start = usize(zip64_eocdr_locator.zip64_eocdr_offset)?;
            let zip64_eocdr_search_end = eocdr_posit
                .checked_sub(spec::Zip64EndOfCentralDirectoryLocator::size_in_file())
                .ok_or(ZipError::InvalidArchive(
                    "Too small for Zip64 End Of Central Directory Record",
                ))?;
            let zip64_eocdr_search_space =
                &mapping[zip64_eocdr_search_start..zip64_eocdr_search_end];

            let zip64_eocdr_posit = spec::find_zip64_eocdr(zip64_eocdr_search_space)?;

            archive_offset = zip64_eocdr_posit;
            let zip64_eocdr = spec::Zip64EndOfCentralDirectory::parse(
                &zip64_eocdr_search_space[zip64_eocdr_posit..],
            )?;

            trace!("{:?}", zip64_eocdr);

            nominal_central_directory_offset = usize(zip64_eocdr.central_directory_offset)?;
            entry_count = zip64_eocdr.entries;
        } else {

            let actual_cdr_posit = eocdr_posit.checked_sub(usize(eocdr.central_directory_size)?);
            let nominal_offset = usize(eocdr.central_directory_offset)?;
            archive_offset = actual_cdr_posit
                .and_then(|off| off.checked_sub(nominal_offset))
                .ok_or(ZipError::InvalidArchive(
                    "Invalid central directory size or offset",
                ))?;
            nominal_central_directory_offset = usize(eocdr.central_directory_offset)?;
            entry_count = eocdr.entries as u64;
        }

        mapping = &mapping[archive_offset..];
        trace!(
            "{} entries at nominal offset {}",
            entry_count,
            nominal_central_directory_offset
        );

        let mut central_directory = &mapping[nominal_central_directory_offset..];

        let mut entries = Vec::new();
        entries.reserve(usize(entry_count)?);

        for _ in 0..entry_count {
            let dir_entry = spec::CentralDirectoryEntry::parse_and_consume(&mut central_directory)?;
            trace!("{:?}", dir_entry);

            let file_metadata = FileMetadata::from_cde(&dir_entry)?;
            debug!("{:?}", file_metadata);
            entries.push(file_metadata);
        }

        Ok((ZipArchive { mapping, entries }, archive_offset))
    }

    pub fn entries(&self) -> &[FileMetadata] {
        &self.entries
    }

    pub fn read(&self, metadata: &FileMetadata) -> ZipResult<Box<dyn io::Read + Send + 'a>> {
        let mut file_slice = &self.mapping[metadata.header_offset..];
        let local_header = spec::LocalFileHeader::parse_and_consume(&mut file_slice)?;
        trace!("{:?}", local_header);
        let local_metadata =
            FileMetadata::from_local_header(&local_header, metadata)?;
        debug!("Reading {:?}", local_metadata);
        if cfg!(feature = "check-local-metadata") && *metadata != local_metadata {
            return Err(ZipError::InvalidArchive(
                "Central directory entry doesn't match local file header",
            ));
        }

        if metadata.encrypted {
            return Err(ZipError::UnsupportedArchive(format!(
                "Can't read encrypted file {}",
                metadata.path
            )));
        }

        make_reader(
            metadata.compression_method,
            metadata.crc32,
            io::Cursor::new(&file_slice[0..metadata.compressed_size]),
        )
    }
}

fn make_reader<'a, R: io::Read + Send + 'a>(
    compression_method: CompressionMethod,
    crc32: u32,
    reader: R,
) -> ZipResult<Box<dyn io::Read + Send + 'a>> {
    match compression_method {
        CompressionMethod::None => Ok(Box::new(Crc32Reader::new(reader, crc32))),
        CompressionMethod::Deflate => {
            let deflate_reader = DeflateDecoder::new(reader);
            Ok(Box::new(Crc32Reader::new(deflate_reader, crc32)))
        }
        _ => Err(ZipError::UnsupportedArchive(String::from(
            "Compression method not supported",
        ))),
    }
}

pub type DirectoryContents<'a> = BTreeMap<&'a str, DirectoryEntry<'a>>;

#[derive(Debug)]
pub struct Directory<'a> {
    pub metadata: &'a FileMetadata<'a>,
    pub children: DirectoryContents<'a>,
}

impl<'a> Directory<'a> {
    fn new(metadata: &'a FileMetadata<'a>) -> Self {
        Self {
            metadata,
            children: DirectoryContents::new(),
        }
    }
}

#[derive(Debug)]
pub enum DirectoryEntry<'a> {
    File(&'a FileMetadata<'a>),
    Directory(Directory<'a>),
}

impl<'a> DirectoryEntry<'a> {

    pub fn metadata(&self) -> &'a FileMetadata<'a> {
        match &self {
            DirectoryEntry::File(metadata) => metadata,
            DirectoryEntry::Directory(dir) => dir.metadata,
        }
    }

    fn name(&self) -> &'a str {
        let path = &self.metadata().path;
        path.file_name().expect("Path ended in ..")
    }
}

pub fn as_tree<'a>(entries: &'a [FileMetadata<'a>]) -> ZipResult<DirectoryContents<'a>> {
    let mut contents = DirectoryContents::new();

    for entry in entries {
        entree_entry(entry, &mut contents)?;
    }

    Ok(contents)
}

pub trait FileTree<'a> {

    fn lookup<P: AsRef<Utf8Path>>(&self, path: P) -> ZipResult<&'a FileMetadata<'a>>;

    fn traverse<'b>(&'b self) -> TreeIterator<'a, 'b>;

    fn files<'b>(&'b self) -> FileTreeIterator<'a, 'b>;

    fn directories<'b>(&'b self) -> DirectoryTreeIterator<'a, 'b>;
}

impl<'a> FileTree<'a> for DirectoryContents<'a> {
    fn lookup<P: AsRef<Utf8Path>>(&self, path: P) -> ZipResult<&'a FileMetadata<'a>> {
        let path = path.as_ref();
        let parent_dir = if let Some(parent) = path.parent() {
            match walk_parent_directories(parent, self) {
                Err(ZipError::NoSuchFile(_)) => Err(ZipError::NoSuchFile(path.to_owned())),
                other_result => other_result,
            }?
        } else {
            self
        };

        let base = path
            .file_name()
            .ok_or_else(|| ZipError::InvalidPath(format!("Path {} ended in ..", path)))?;

        parent_dir
            .get(base)
            .ok_or_else(|| ZipError::NoSuchFile(path.to_owned()))
            .map(|dir_entry| dir_entry.metadata())
    }

    fn traverse<'b>(&'b self) -> TreeIterator<'a, 'b> {
        TreeIterator::new(self)
    }

    fn files<'b>(&'b self) -> FileTreeIterator<'a, 'b> {
        FileTreeIterator::new(self)
    }

    fn directories<'b>(&'b self) -> DirectoryTreeIterator<'a, 'b> {
        DirectoryTreeIterator::new(self)
    }
}

fn entree_entry<'a>(
    entry: &'a FileMetadata<'a>,
    tree: &mut DirectoryContents<'a>,
) -> ZipResult<()> {
    let path = &entry.path;

    let parent_dir = if let Some(parent) = path.parent() {
        walk_parent_directories_mut(parent, tree)?
    } else {
        tree
    };

    let _base = path
        .file_name()
        .ok_or_else(|| ZipError::Hierarchy(format!("Path {path} ended in ..")))?;

    let to_insert: DirectoryEntry = if entry.is_dir() {
        DirectoryEntry::Directory(Directory::new(entry))
    } else {
        DirectoryEntry::File(entry)
    };

    if parent_dir.insert(to_insert.name(), to_insert).is_some() {
        return Err(ZipError::Hierarchy(format!("Duplicate entry for {path}",)));
    }

    Ok(())
}

fn walk_parent_directories_mut<'a, 'b>(
    path: &Utf8Path,
    tree: &'b mut DirectoryContents<'a>,
) -> ZipResult<&'b mut DirectoryContents<'a>> {
    let mut current = tree;

    for component in path.components() {
        match component {
            Utf8Component::Prefix(prefix) => {
                let prefix = prefix.as_os_str();
                return Err(ZipError::Hierarchy(format!(
                    "Prefix {} found in path {path}",
                    prefix.to_string_lossy(),
                )));
            }
            Utf8Component::RootDir => {
                warn!("Root directory found in path {path}");

            }
            Utf8Component::CurDir => {
                warn!("Current dir (.) found in path {path}");

            }
            Utf8Component::ParentDir => {

                return Err(ZipError::Hierarchy(format!(
                    "Parent dir (..) found in path {path}",
                )));
            }

            Utf8Component::Normal(component) => {
                if let Some(child) = current.get_mut(component) {
                    match child {
                        DirectoryEntry::Directory(dir) => {
                            current = &mut dir.children;
                        }
                        _ => {
                            return Err(ZipError::Hierarchy(format!(
                                "{path} is a file, expected a directory",
                            )));
                        }
                    }
                } else {
                    return Err(ZipError::Hierarchy(format!(
                        "{path} found before parent directories",
                    )));
                }
            }
        }
    }
    Ok(current)
}

fn walk_parent_directories<'a, 'b>(
    path: &Utf8Path,
    tree: &'b DirectoryContents<'a>,
) -> ZipResult<&'b DirectoryContents<'a>> {
    let mut current = tree;

    for component in path.components() {

        match component {
            Utf8Component::Prefix(prefix) => {
                return Err(ZipError::InvalidPath(format!(
                    "Prefix {prefix} found in path {path}",
                )));
            }
            Utf8Component::RootDir => {
                return Err(ZipError::InvalidPath(format!(
                    "Root directory found in path {path}",
                )));
            }
            Utf8Component::CurDir => {
                return Err(ZipError::InvalidPath(format!(
                    "Current dir (.) found in path {path}",
                )));
            }
            Utf8Component::ParentDir => {
                return Err(ZipError::InvalidPath(format!(
                    "Parent dir (..) found in path {path}",
                )));
            }

            Utf8Component::Normal(component) => {
                if let Some(child) = current.get(component) {
                    match child {
                        DirectoryEntry::Directory(dir) => {
                            current = &dir.children;
                        }
                        _ => {
                            return Err(ZipError::InvalidPath(format!(
                                "{path} is a file, expected a directory",
                            )));
                        }
                    }
                } else {
                    return Err(ZipError::NoSuchFile(path.to_owned()));
                }
            }
        }
    }
    Ok(current)
}

pub struct TreeIterator<'a, 'b> {
    stack: Vec<btree_map::Values<'b, &'a str, DirectoryEntry<'a>>>,
}

impl<'a, 'b> TreeIterator<'a, 'b> {
    fn new(tree: &'b DirectoryContents<'a>) -> Self {
        let stack = vec![tree.values()];
        Self { stack }
    }
}

impl<'a, 'b> Iterator for TreeIterator<'a, 'b> {
    type Item = &'b DirectoryEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stack.is_empty() {
            return None;
        }
        let next = self.stack.last_mut().unwrap().next();
        match next {
            Some(entry) => {
                if let DirectoryEntry::Directory(d) = entry {
                    self.stack.push(d.children.values());
                }
                return Some(entry);
            }
            None => {
                self.stack.pop();
            }
        };
        self.next()
    }
}

pub struct FileTreeIterator<'a, 'b> {
    inner: TreeIterator<'a, 'b>,
}

impl<'a, 'b> FileTreeIterator<'a, 'b> {
    fn new(tree: &'b DirectoryContents<'a>) -> Self {
        Self {
            inner: TreeIterator::new(tree),
        }
    }
}

impl<'a> Iterator for FileTreeIterator<'a, '_> {
    type Item = &'a FileMetadata<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.inner.stack.is_empty() {
            return None;
        }
        let next = self.inner.stack.last_mut().unwrap().next();
        match next {
            Some(DirectoryEntry::File(f)) => {
                return Some(f);
            }
            Some(DirectoryEntry::Directory(d)) => {
                self.inner.stack.push(d.children.values());
            }
            None => {
                self.inner.stack.pop();
            }
        };
        self.next()
    }
}

pub struct DirectoryTreeIterator<'a, 'b> {
    inner: TreeIterator<'a, 'b>,
}

impl<'a, 'b> DirectoryTreeIterator<'a, 'b> {
    fn new(tree: &'b DirectoryContents<'a>) -> Self {
        Self {
            inner: TreeIterator::new(tree),
        }
    }
}

impl<'a, 'b> Iterator for DirectoryTreeIterator<'a, 'b> {
    type Item = &'b Directory<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.inner.stack.is_empty() {
            return None;
        }
        let next = self.inner.stack.last_mut().unwrap().next();
        match next {
            Some(DirectoryEntry::Directory(d)) => {
                self.inner.stack.push(d.children.values());
                return Some(d);
            }
            Some(DirectoryEntry::File(_f)) => {}
            None => {
                self.inner.stack.pop();
            }
        };
        self.next()
    }
}
