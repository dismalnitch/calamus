//! Shared helpers for integration tests.

use calamus::buffer::Buffer;

/// A buffer pre-filled with `text`.
pub fn buf(text: &str) -> Buffer {
    let mut b = Buffer::new();
    b.insert(0, text).expect("insert into empty buffer at 0");
    b
}

/// Asserts the buffer contains exactly `expected`, no more and no less.
#[track_caller]
pub fn assert_text(b: &Buffer, expected: &str) {
    let n = expected.chars().count();
    let actual = b.read(0, n).map(|s| s.text.to_string());
    assert_eq!(actual.as_deref(), Ok(expected), "buffer prefix mismatch");
    assert!(b.read(0, n + 1).is_err(), "buffer is longer than {expected:?}");
}
