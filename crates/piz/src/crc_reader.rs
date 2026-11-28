use std::io;
use std::io::prelude::*;

use crc32fast::Hasher;

pub struct Crc32Reader<R> {
    inner: R,
    hasher: Hasher,
    provided_checksum: u32,
}

impl<R> Crc32Reader<R> {
    pub fn new(inner: R, provided_checksum: u32) -> Crc32Reader<R> {
        Crc32Reader {
            inner,
            hasher: Hasher::new(),
            provided_checksum,
        }
    }

    fn check_matches(&self) -> bool {
        self.provided_checksum == self.hasher.clone().finalize()
    }
}

impl<R: Read> Read for Crc32Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = match self.inner.read(buf) {
            Ok(0) if !buf.is_empty() && !self.check_matches() => {
                return Err(io::Error::new(io::ErrorKind::Other, "Invalid checksum"))
            }
            Ok(n) => n,
            Err(e) => return Err(e),
        };
        self.hasher.update(&buf[0..count]);
        Ok(count)
    }
}
