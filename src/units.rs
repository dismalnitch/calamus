//! units

pub enum Heading {
    North,
    East,
    West,
    South,
}

pub struct Offset {
    pub offset: usize,
    pub heading: Heading,
}
