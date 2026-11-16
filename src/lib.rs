pub mod buffer;
pub mod caret;
pub mod display;
pub mod layout;
pub mod units;

use buffer::Buffer;

pub struct Editor {
    cursor: usize,
    buffer: Buffer,
}

// TODO: implement
// impl Editor {
//     fn next_cluster(&self, at: Offset) -> Offset;
//     fn to_utf16(&self, at: Offset) -> u32;
//     fn from_utf16(&self, units: u32) -> Offset;
// }
