use std::ops::{BitOr, BitOrAssign};

/// Codec directions: `encode` is Rust → JS (`Serialize`), `decode` is JS → Rust
/// (`Deserialize`).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Directions {
    pub(crate) encode: bool,
    pub(crate) decode: bool,
}

impl Directions {
    pub(crate) const NONE: Self = Self {
        encode: false,
        decode: false,
    };
    pub(crate) const ENCODE: Self = Self {
        encode: true,
        decode: false,
    };
    pub(crate) const DECODE: Self = Self {
        encode: false,
        decode: true,
    };
    pub(crate) const BOTH: Self = Self {
        encode: true,
        decode: true,
    };

    pub(crate) fn intersects(self, other: Self) -> bool {
        (self.encode && other.encode) || (self.decode && other.decode)
    }
}

impl BitOr for Directions {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            encode: self.encode || other.encode,
            decode: self.decode || other.decode,
        }
    }
}

impl BitOrAssign for Directions {
    fn bitor_assign(&mut self, other: Self) {
        *self = *self | other;
    }
}
