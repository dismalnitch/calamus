use crate::common::{assert_text, buf};
use calamus::buffer::{Buffer, BufferError};

mod insert {
    use super::*;

    #[test]
    fn into_empty() {
        let mut b = Buffer::new();
        assert_eq!(b.insert(0, "hello"), Ok(5));
        assert_text(&b, "hello");
    }

    #[test]
    fn counts_bytes_not_chars() {
        let mut b = Buffer::new();
        assert_eq!(b.insert(0, "héllo 👍"), Ok(11));
    }

    #[test]
    fn inside_a_char_errors_and_leaves_buffer_alone() {
        let mut b = buf("é👍");
        assert_eq!(b.insert(1, "x"), Err(BufferError::NotCharBoundary { at: 1 }));
        assert_eq!(b.insert(4, "x"), Err(BufferError::NotCharBoundary { at: 4 }));
        assert_text(&b, "é👍");
    }

    #[test]
    fn between_multibyte_chars() {
        let mut b = buf("é👍");
        assert_eq!(b.insert(2, "x"), Ok(1));
        assert_text(&b, "éx👍");
    }

    #[test]
    fn at_end() {
        let mut b = buf("ab");
        assert_eq!(b.insert(2, "c"), Ok(1));
        assert_text(&b, "abc");
    }

    #[test]
    fn past_end_errors_and_leaves_buffer_alone() {
        let mut b = buf("ab");
        assert_eq!(b.insert(3, "x"), Err(BufferError::OutOfBounds { at: 3, len: 2 }));
        assert_text(&b, "ab");
    }
}

mod read {
    use super::*;

    #[test]
    fn middle() {
        let b = buf("hello world");
        let s = b.read(6, 5).unwrap();
        assert_eq!(s.text.to_string(), "world");
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn empty_read_at_end() {
        let b = buf("abc");
        assert!(b.read(3, 0).unwrap().is_empty());
    }

    #[test]
    fn past_end_errors() {
        let b = buf("abc");
        assert_eq!(
            b.read(2, 5).err(),
            Some(BufferError::RangeOutOfBounds { at: 2, end: 7, len: 3 })
        );
    }

    #[test]
    fn inside_a_char_errors() {
        let b = buf("aé✓");
        assert_eq!(b.read(2, 1).err(), Some(BufferError::NotCharBoundary { at: 2 }));
        assert_eq!(b.read(1, 3).err(), Some(BufferError::NotCharBoundary { at: 4 }));
        assert_eq!(b.read(1, 2).unwrap().text.to_string(), "é");
    }

    #[test]
    fn huge_len_does_not_overflow() {
        let b = buf("abc");
        assert!(b.read(1, usize::MAX).is_err());
    }
}

mod replace {
    use super::*;

    #[test]
    fn delete() {
        let mut b = buf("hello");
        assert_eq!(b.replace((0..2).into(), ""), Ok(0));
        assert_text(&b, "llo");
    }

    #[test]
    fn middle() {
        let mut b = buf("hello");
        assert_eq!(b.replace((1..4).into(), "EY👍"), Ok(6));
        assert_text(&b, "hEY👍o");
    }

    #[test]
    fn empty_range_inserts() {
        let mut b = buf("ab");
        assert_eq!(b.replace((1..1).into(), "X"), Ok(1));
        assert_text(&b, "aXb");
    }

    #[test]
    fn whole_buffer() {
        let mut b = buf("abc");
        assert_eq!(b.replace((0..3).into(), "z"), Ok(1));
        assert_text(&b, "z");
    }

    #[test]
    fn past_end_errors_and_leaves_buffer_alone() {
        let mut b = buf("abc");
        assert_eq!(
            b.replace((1..9).into(), "x"),
            Err(BufferError::RangeOutOfBounds { at: 1, end: 9, len: 3 })
        );
        assert_text(&b, "abc");
    }

    #[test]
    fn inside_a_char_errors_and_leaves_buffer_alone() {
        let mut b = buf("a👍b");
        assert_eq!(
            b.replace((0..2).into(), "x"),
            Err(BufferError::NotCharBoundary { at: 2 })
        );
        assert_eq!(
            b.replace((3..5).into(), "x"),
            Err(BufferError::NotCharBoundary { at: 3 })
        );
        assert_text(&b, "a👍b");
        assert_eq!(b.replace((1..5).into(), "é"), Ok(2));
        assert_text(&b, "aéb");
    }

    #[test]
    #[allow(clippy::reversed_empty_ranges)]
    fn backwards_range_errors_and_leaves_buffer_alone() {
        let mut b = buf("abc");
        assert!(b.replace((2..1).into(), "x").is_err());
        assert_text(&b, "abc");
    }
}
