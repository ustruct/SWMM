use std::io::Read;

fn archive_with_local_zip64_offset() -> Vec<u8> {
    let name = b"hello.txt";
    let data = b"hello";
    let crc = crc32fast::hash(data);
    let date: u16 = (1 << 5) | 1;

    let mut extra = Vec::new();
    extra.extend_from_slice(&1u16.to_le_bytes());
    extra.extend_from_slice(&8u16.to_le_bytes());
    extra.extend_from_slice(&0u64.to_le_bytes());

    let mut out = Vec::new();
    out.extend_from_slice(&0x04034b50u32.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(&extra);
    out.extend_from_slice(data);

    let cd_offset = out.len() as u32;
    out.extend_from_slice(&0x02014b50u32.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(name);
    let cd_size = out.len() as u32 - cd_offset;

    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[test]
fn local_zip64_field_with_only_an_offset_opens() {
    let bytes = archive_with_local_zip64_offset();
    let archive = piz::ZipArchive::new(&bytes).unwrap();
    let entry = &archive.entries()[0];
    let mut text = String::new();
    archive.read(entry).unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "hello");
}
