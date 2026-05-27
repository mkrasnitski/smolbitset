use crate::{BITS, SmolBitSet};

use core::fmt;

#[cfg(not(feature = "std"))]
use extern_alloc::vec::Vec;

use fmt::{Binary, Debug, Display, Formatter, LowerHex, Octal, Result, UpperHex};

impl Debug for SmolBitSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if self.is_sparse() {
            return Debug::fmt(&self.as_normal(), f);
        }

        f.debug_list().entries(self.data().iter()).finish()
    }
}

impl Display for SmolBitSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if self.is_sparse() {
            return Display::fmt(&self.as_normal(), f);
        }

        let slice = self.data();
        if slice.len() == 1 {
            write!(f, "{}", slice[0])
        } else {
            let bytes = slice
                .iter()
                .flat_map(|block| block.to_le_bytes().into_iter())
                .collect::<Vec<_>>();
            write!(f, "{}", num_bigint::BigUint::from_bytes_le(&bytes))
        }
    }
}

macro_rules! impl_format {
    ($($kind:ident $format:literal $variants:literal),+) => {$(
        impl $kind for SmolBitSet {
            fn fmt(&self, f: &mut Formatter<'_>) -> Result {
                const PAD: usize = BITS / ($variants as u8).ilog2() as usize;

                if self.is_sparse() {
                    return $kind::fmt(&self.as_normal(), f);
                }

                let cap = self.highest_set_bit().unwrap_or_default() + 1;

                let mut full_width = false;
                for d in self.data().iter().take(cap.div_ceil(BITS)).rev() {
                    if full_width {
                        write!(f, concat!("{:0PADDING$", $format, "}"), d, PADDING = PAD)?;
                    } else {
                        full_width = true;
                        $kind::fmt(&d, f)?;
                    }
                }

                Ok(())
            }
        }
    )*};
}

impl_format!(
    Binary 'b' 2,
    Octal 'o' 8,
    LowerHex 'x' 16,
    UpperHex 'X' 16
);
