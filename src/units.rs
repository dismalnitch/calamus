//! units

pub enum Direction {
    N,
    E,
    W,
    S,
}

pub struct Offset {
    offset: usize,
    due: Direction,
}
