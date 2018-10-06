use super::{copy_encode, decode_all, encode_all};
use super::{Decoder, Encoder};

use partial_io::{PartialOp, PartialWrite};

use std::io;
use std::iter;

#[test]
fn test_end_of_frame() {
    use std::io::{Read, Write};

    let mut enc = Encoder::new(Vec::new(), 1).unwrap();
    enc.write_all(b"foo").unwrap();
    let mut compressed = enc.finish().unwrap();

    // Add footer/whatever to underlying storage.
    compressed.push(0);

    // Drain zstd stream until end-of-frame.
    let mut dec = Decoder::new(&compressed[..]).unwrap().single_frame();
    let mut buf = Vec::new();
    dec.read_to_end(&mut buf).unwrap();
    assert_eq!(&buf, b"foo");
}

#[test]
fn test_concatenated_frames() {
    let mut buffer = Vec::new();
    copy_encode(&b"foo"[..], &mut buffer, 1).unwrap();
    copy_encode(&b"bar"[..], &mut buffer, 2).unwrap();
    copy_encode(&b"baz"[..], &mut buffer, 3).unwrap();

    assert_eq!(&decode_all(&buffer[..]).unwrap(), b"foobarbaz");
}

#[test]
fn test_flush() {
    use std::io::Write;

    let buf = Vec::new();
    let mut z = Encoder::new(buf, 19).unwrap();

    z.write_all(b"hello").unwrap();

    z.flush().unwrap(); // Might corrupt stream
    let buf = z.finish().unwrap();

    let s = decode_all(&buf[..]).unwrap();
    let s = ::std::str::from_utf8(&s).unwrap();
    assert_eq!(s, "hello");
}

#[test]
fn test_try_finish() {
    use std::io::Write;
    let mut z = setup_try_finish();

    z.get_mut().set_ops(iter::repeat(PartialOp::Unlimited));

    // flush() should continue to work even though write() doesn't.
    z.flush().unwrap();

    let buf = match z.try_finish() {
        Ok(buf) => buf.into_inner(),
        Err((_z, e)) => panic!("try_finish failed with {:?}", e),
    };

    // Make sure the multiple try_finish calls didn't screw up the internal
    // buffer and continued to produce valid compressed data.
    assert_eq!(&decode_all(&buf[..]).unwrap(), b"hello");
}

#[test]
#[should_panic]
fn test_write_after_try_finish() {
    use std::io::Write;
    let mut z = setup_try_finish();
    z.write_all(b"hello world").unwrap();
}

fn setup_try_finish() -> Encoder<PartialWrite<Vec<u8>>> {
    use std::io::Write;

    let buf =
        PartialWrite::new(Vec::new(), iter::repeat(PartialOp::Unlimited));
    let mut z = Encoder::new(buf, 19).unwrap();

    z.write_all(b"hello").unwrap();

    z.get_mut()
        .set_ops(iter::repeat(PartialOp::Err(io::ErrorKind::WouldBlock)));

    let (z, err) = z.try_finish().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

    z
}

#[test]
fn test_failing_write() {
    use std::io::Write;

    let buf = PartialWrite::new(
        Vec::new(),
        iter::repeat(PartialOp::Err(io::ErrorKind::WouldBlock)),
    );
    let mut z = Encoder::new(buf, 1).unwrap();

    // Fill in enough data to make sure the buffer gets written out.
    let input = "b".repeat(128 * 1024).into_bytes();
    // This should work even though the inner writer rejects writes.
    assert_eq!(z.write(&input).unwrap(), 128 * 1024);

    // The next write would fail (the buffer still has some data in it).
    assert_eq!(
        z.write(b"abc").unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );

    z.get_mut().set_ops(iter::repeat(PartialOp::Unlimited));

    // This shouldn't have led to any corruption.
    let buf = z.finish().unwrap().into_inner();
    assert_eq!(&decode_all(&buf[..]).unwrap(), &input);
}

#[test]
fn test_invalid_frame() {
    use std::io::Read;

    // I really hope this data is invalid...
    let data = &[1u8, 2u8, 3u8, 4u8, 5u8];
    let mut dec = Decoder::new(&data[..]).unwrap();
    assert_eq!(
        dec.read_to_end(&mut Vec::new()).err().map(|e| e.kind()),
        Some(io::ErrorKind::Other)
    );
}

#[test]
fn test_incomplete_frame() {
    use std::io::{Read, Write};

    let mut enc = Encoder::new(Vec::new(), 1).unwrap();
    enc.write_all(b"This is a regular string").unwrap();
    let mut compressed = enc.finish().unwrap();

    let half_size = compressed.len() - 2;
    compressed.truncate(half_size);

    let mut dec = Decoder::new(&compressed[..]).unwrap();
    assert_eq!(
        dec.read_to_end(&mut Vec::new()).err().map(|e| e.kind()),
        Some(io::ErrorKind::UnexpectedEof)
    );
}

#[test]
fn test_legacy() {
    use std::fs;
    use std::io::Read;

    let mut target = Vec::new();

    // Read the content from that file
    fs::File::open("assets/example.txt")
        .unwrap()
        .read_to_end(&mut target)
        .unwrap();

    for version in &[5, 6, 7, 8] {
        let filename = format!("assets/example.txt.v{}.zst", version);
        let file = fs::File::open(filename).unwrap();
        let mut decoder = Decoder::new(file).unwrap();

        let mut buffer = Vec::new();
        decoder.read_to_end(&mut buffer).unwrap();

        assert!(
            target == buffer,
            "Error decompressing legacy version {}",
            version
        );
    }
}

// Check that compressing+decompressing some data gives back the original
fn test_full_cycle(input: &[u8], level: i32) {
    ::test_cycle_unwrap(
        input,
        |data| encode_all(data, level),
        |data| decode_all(data),
    );
}

#[test]
fn test_ll_source() {
    // Where could I find some long text?...
    let data = include_bytes!("../../zstd-safe/zstd-sys/src/bindings.rs");
    // Test a few compression levels.
    // TODO: check them all?
    for level in 1..5 {
        test_full_cycle(data, level);
    }
}