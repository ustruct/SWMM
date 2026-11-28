use crate::result::*;

pub fn usize<I: Into<u64>>(i: I) -> ZipResult<usize> {
    let i: u64 = i.into();
    if cfg!(target_pointer_width = "64") {
        Ok(i as usize)
    } else if i > usize::MAX as u64 {
        Err(ZipError::InsufficientAddressSpace)
    } else {
        Ok(i as usize)
    }
}
