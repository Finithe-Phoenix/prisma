//! A minimal x86-64 ET_EXEC with an RX load segment and a small RW segment.

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn image(base: u64, code: &[u8]) -> Vec<u8> {
    assert!(
        code.len() <= 0x1000 - 0xe0,
        "code must fit the text segment"
    );
    let mut bytes = vec![0; 0x1003];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put16(&mut bytes, 16, 2);
    put16(&mut bytes, 18, 62);
    put32(&mut bytes, 20, 1);
    put64(&mut bytes, 24, base + 0xe0);
    put64(&mut bytes, 32, 64);
    put16(&mut bytes, 52, 64);
    put16(&mut bytes, 54, 56);
    put16(&mut bytes, 56, 2);
    for (index, flags, offset, va, filesz, memsz) in [
        (
            0,
            5,
            0,
            base,
            0xe0 + code.len() as u64,
            0xe0 + code.len() as u64,
        ),
        (1, 6, 0x1000, base + 0x1000, 3, 64),
    ] {
        let ph = 64 + index * 56;
        put32(&mut bytes, ph, 1);
        put32(&mut bytes, ph + 4, flags);
        put64(&mut bytes, ph + 8, offset);
        put64(&mut bytes, ph + 16, va);
        put64(&mut bytes, ph + 32, filesz);
        put64(&mut bytes, ph + 40, memsz);
        put64(&mut bytes, ph + 48, 0x1000);
    }
    bytes[0xe0..0xe0 + code.len()].copy_from_slice(code);
    bytes[0x1000..].copy_from_slice(b"abc");
    bytes
}
