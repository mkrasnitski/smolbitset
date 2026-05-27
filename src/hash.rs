use crate::{BITS, SmolBitSet};

use core::hash;

impl hash::Hash for SmolBitSet {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        if self.is_sparse() {
            self.as_normal().hash(state);
            return;
        }

        let cap = self.highest_set_bit().unwrap_or_default() + 1;
        for d in self.data().iter().take(cap.div_ceil(BITS)) {
            d.hash(state);
        }
    }
}

// core does not have a default hasher
#[cfg(all(test, feature = "std"))]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn hash_helper(a: &SmolBitSet) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        a.hash(&mut hasher);
        hasher.finish()
    }

    fn assert_hash_eq(a: &SmolBitSet, b: &SmolBitSet) {
        assert_eq!(hash_helper(a), hash_helper(b));
    }

    fn assert_hash_ne(a: &SmolBitSet, b: &SmolBitSet) {
        assert_ne!(hash_helper(a), hash_helper(b));
    }

    #[test]
    fn normal_inline() {
        let val = 0xC5C5_F00D;
        let mut a = SmolBitSet::new_inline(val);
        let mut b = SmolBitSet::new_inline(val);
        assert_hash_eq(&a, &b);

        a >>= 1u8;
        b >>= 4u8;
        assert_hash_ne(&a, &b);

        a >>= 3u8;
        assert_hash_eq(&a, &b);
    }

    #[test]
    fn sparse_inline() {
        let flag = 1337;
        let mut a = SmolBitSet::flag(flag);
        let mut b = SmolBitSet::flag(flag);
        assert_hash_eq(&a, &b);

        a <<= 42u8;
        b <<= 66u8;
        assert_hash_ne(&a, &b);

        a <<= 24u8;
        assert_hash_eq(&a, &b);
    }

    #[test]
    fn mixed_inline() {
        let mut a = SmolBitSet::new_inline(1 << 15);
        let mut b = SmolBitSet::flag(15);
        assert_hash_eq(&a, &b);

        b <<= 420u16;
        a >>= 5u8;
        assert_hash_ne(&a, &b);

        b >>= 425u16;
        assert_hash_eq(&a, &b);
    }

    #[test]
    fn normal_heap() {
        let val = 0xFFC5_C0FF_EE00_BEEF_u64;
        let mut a = SmolBitSet::from(val);
        let mut b = SmolBitSet::from(val);
        assert!(!a.is_inline());
        assert!(!b.is_inline());
        assert_hash_eq(&a, &b);

        a <<= 128u8;
        a >>= 66u8;
        assert_hash_ne(&a, &b);

        b <<= 128u8 - 66u8;
        assert_hash_eq(&a, &b);
    }

    #[test]
    fn normal_mixed() {
        let val = 0xBEEF_F00D;
        let mut a = SmolBitSet::new_inline(val);
        let mut b = SmolBitSet::new_inline(val);
        b.reserve(64);
        assert!(a.is_inline());
        assert!(!b.is_inline());
        assert_hash_eq(&a, &b);

        a >>= 5u8;
        assert_hash_ne(&a, &b);

        b >>= 5u8;
        assert_hash_eq(&a, &b);
    }
}
