use crate::{Representation, SmolBitSet};

use core::{cmp, iter};

impl cmp::PartialEq for SmolBitSet {
    fn eq(&self, other: &Self) -> bool {
        match (self.representation(), other.representation()) {
            (Representation::Sparse, Representation::Sparse) => {
                let this = unsafe { self.get_sparse_data_unchecked() };
                let other = unsafe { other.get_sparse_data_unchecked() };
                this == other
            }
            (Representation::Sparse, Representation::Inline | Representation::Alloc) => {
                self.as_normal() == *other
            }
            (Representation::Inline | Representation::Alloc, Representation::Sparse) => {
                *self == other.as_normal()
            }
            (
                Representation::Alloc | Representation::Inline,
                Representation::Alloc | Representation::Inline,
            ) => {
                let this = self.data();
                let other = other.data();

                let (long, short) = if this.len() >= other.len() {
                    (this, other)
                } else {
                    (other, this)
                };

                let (prefix, suffix) = long.split_at(short.len());
                prefix == short.as_ref() && suffix.iter().all(|&x| x == 0)
            }
        }
    }
}

impl cmp::Eq for SmolBitSet {}

impl cmp::PartialOrd for SmolBitSet {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl cmp::Ord for SmolBitSet {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        fn slice_cmp(long: &[usize], short: &[usize]) -> cmp::Ordering {
            let (prefix, suffix) = long.split_at(short.len());
            if suffix.iter().any(|&x| x != 0) {
                return cmp::Ordering::Greater;
            }

            for (a, b) in iter::zip(prefix, short).rev() {
                let cmp = a.cmp(b);
                if cmp != cmp::Ordering::Equal {
                    return cmp;
                }
            }

            cmp::Ordering::Equal
        }

        match (self.representation(), other.representation()) {
            (Representation::Sparse, Representation::Sparse) => {
                let this = unsafe { self.get_sparse_data_unchecked() };
                let other = unsafe { other.get_sparse_data_unchecked() };
                this.cmp(&other)
            }
            (Representation::Sparse, Representation::Inline | Representation::Alloc) => {
                self.as_normal().cmp(other)
            }
            (Representation::Inline | Representation::Alloc, Representation::Sparse) => {
                self.cmp(&other.as_normal())
            }
            (
                Representation::Alloc | Representation::Inline,
                Representation::Alloc | Representation::Inline,
            ) => {
                let this = self.data();
                let other = other.data();

                if this.len() >= other.len() {
                    slice_cmp(this.as_ref(), other.as_ref())
                } else {
                    slice_cmp(other.as_ref(), this.as_ref()).reverse()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod normal {
        use super::*;

        #[test]
        fn eq() {
            let mut a = SmolBitSet::from(u16::MAX);
            let mut b = SmolBitSet::from(0xFFFFu16);
            assert_eq!(a, b);

            a <<= 55;
            assert_ne!(a, b);

            b <<= 55;
            assert_eq!(a, b);
        }

        #[test]
        fn ord() {
            let mut a = SmolBitSet::from(0xBEEFu16);
            let mut b = SmolBitSet::from(0x00C5_F00Du32);
            assert!(a < b);

            a <<= 72;
            assert!(a > b);
            assert!(b < a);

            b <<= 72;
            assert!(a < b);
        }

        #[test]
        fn eq_but_not_physical_same() {
            let mut a = SmolBitSet::from(u16::MAX);
            let mut b = SmolBitSet::from(0xFFFFu16);

            // ensure a is larger than b in memory
            a.reserve(256);

            assert_eq!(a, b);
            assert_eq!(b, a);

            a <<= 55;
            assert_ne!(a, b);
            assert_ne!(b, a);

            b <<= 55;
            assert_eq!(a, b);
            assert_eq!(b, a);
        }

        #[test]
        fn ord_but_not_physical_same() {
            let mut a = SmolBitSet::from(0xBEEFu16);
            let mut b = SmolBitSet::from(0x00C5_F00Du32);

            // ensure a is larger than b in memory
            a.reserve(256);

            assert!(a < b);
            assert!(b > a);

            a <<= 18;
            assert!(a > b);
            assert!(b < a);

            a <<= 54;
            assert!(a > b);
            assert!(b < a);

            b <<= 72;
            assert!(a < b);
            assert!(b > a);
        }
    }

    mod sparse {
        use super::*;

        #[test]
        fn eq() {
            let mut a = SmolBitSet::flag(12345);
            let mut b = SmolBitSet::flag(12345);
            assert_eq!(a, b);

            a <<= 550;
            assert_ne!(a, b);

            b <<= 550;
            assert_eq!(a, b);
        }

        #[test]
        fn ord() {
            let mut a = SmolBitSet::flag(12345);
            let mut b = SmolBitSet::flag(54321);
            assert!(a < b);

            a <<= 100_000;
            assert!(a > b);
            assert!(b < a);

            b <<= 100_000;
            assert!(a < b);
        }
    }

    mod mixed {
        use super::*;

        #[test]
        fn eq() {
            let mut a = SmolBitSet::new_inline(1 << 22);
            let mut b = SmolBitSet::flag(22);
            assert_eq!(a, b);

            a <<= 32;
            assert_ne!(a, b);

            b <<= 32;
            assert_eq!(a, b);
        }

        #[test]
        fn ord() {
            let mut a = SmolBitSet::new_inline(1);
            let mut b = SmolBitSet::flag(22);
            assert!(a < b);

            a <<= 80;
            assert!(a > b);
            assert!(b < a);

            b <<= 80;
            assert!(a < b);
        }
    }
}
