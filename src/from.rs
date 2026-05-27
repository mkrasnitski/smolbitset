use crate::{BITS, SmolBitSet};

use core::str::FromStr;
use num_bigint::{BigUint, ParseBigIntError};

#[cfg(not(feature = "std"))]
use extern_alloc::{string::String, vec::Vec};

macro_rules! impl_from {
    ($($t:ty),+) => {$(
        impl From<$t> for SmolBitSet {
            fn from(value: $t) -> Self {
                let mut sbs = SmolBitSet::empty();
                sbs.reserve(highest_set_bit!($t, value).map_or(0, |b| b + 1));

                if sbs.is_inline() {
                    unsafe { sbs.write_inline_data_unchecked(value as usize) };
                } else {
                    let data = unsafe { sbs.as_slice_mut_unchecked() };

                    for i in 0..(<$t>::BITS as usize).div_ceil(BITS) {
                        data[i] = (value >> (i * BITS)) as usize;
                    }
                }

                sbs
            }
        }

        impl From<&$t> for SmolBitSet {
            #[inline]
            fn from(value: &$t) -> Self {
                Self::from(*value)
            }
        }
    )*};
}

impl_from!(u8, u16, u32, u64, u128, usize);

impl TryFrom<String> for SmolBitSet {
    type Error = ParseBigIntError;

    #[inline]
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(value.as_str())
    }
}

impl FromStr for SmolBitSet {
    type Err = ParseBigIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tmp = BigUint::from_str(s)?;

        let mut sbs = Self::empty();
        sbs.reserve(tmp.bits() as usize);

        #[cfg(target_pointer_width = "32")]
        let digits = tmp.to_u32_digits();
        #[cfg(target_pointer_width = "64")]
        let digits = tmp.to_u64_digits();

        let digits = digits
            .into_iter()
            .map(|digit| digit as usize)
            .collect::<Vec<_>>();

        if sbs.is_inline() {
            if let Some(&n) = digits.first() {
                unsafe { sbs.write_inline_data_unchecked(n) }
            }
        } else {
            let digit_count = digits.len();
            assert!(sbs.len() >= digit_count);

            let data = unsafe { sbs.as_slice_mut_unchecked() };
            data[0..digit_count].copy_from_slice(&digits);
        }

        Ok(sbs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! test_from {
            ($($name:ident, $val:expr),+) => {$(
                #[test]
                fn $name() {
                    let t = SmolBitSet::from($val);
                    assert!(t.is_inline());

                    let d = unsafe { t.get_inline_data_unchecked() };
                    assert_eq!(d, $val as usize);
                }
            )*}
        }

    test_from! {
        u8, 0b1110_1010u8,
        u8_max, u8::MAX,
        u16, 0xBEAFu16,
        u16_max, u16::MAX,
        u32, 0xF0BA_B0BBu32,
        u32_max, u32::MAX,
        u64, 0x3550_1337_0000_F00Fu64
    }

    #[test]
    fn u64_hb_64() {
        let t = SmolBitSet::from(0xC5C5_BEEF_0000_1234u64);
        assert!(!t.is_inline());
        assert_eq!(t.len(), 1);

        let d = t.data();
        assert_eq!(d.len(), 1);
        assert_eq!(d.as_ref(), [0xC5C5_BEEF_0000_1234]);
    }

    #[test]
    fn u128_max() {
        let t = SmolBitSet::from(u128::MAX);
        assert_eq!(t.len(), 2);

        let d = t.data();
        assert_eq!(d.len(), 2);
        assert_eq!(d.as_ref(), &[u64::MAX as usize; 2]);
    }

    #[test]
    fn parse() {
        let sbs = "0".parse::<SmolBitSet>().unwrap();
        assert_eq!(sbs, SmolBitSet::empty());

        let sbs = "18446744073709551615".parse::<SmolBitSet>().unwrap();
        assert_eq!(sbs, SmolBitSet::from(u64::MAX));

        let sbs = "340282366920938463463374607431768211455"
            .parse::<SmolBitSet>()
            .unwrap();
        assert_eq!(sbs, SmolBitSet::from(u128::MAX));
    }
}
