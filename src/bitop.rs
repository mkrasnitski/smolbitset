use crate::{BITS, Representation, SmolBitSet};

use core::iter;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign};

impl SmolBitSet {
    fn shortening_bitop_assign(
        &mut self,
        other: &Self,
        mut op: impl FnMut(&mut usize, usize),
        sparse_cond: impl Fn(usize, usize) -> bool,
    ) {
        match (self.representation(), other.representation()) {
            (Representation::Sparse, Representation::Sparse) => {
                let flag = unsafe { self.get_sparse_data_unchecked() };
                let other_flag = unsafe { other.get_sparse_data_unchecked() };

                if sparse_cond(flag, other_flag) {
                    // result is still sparse and lhs already has the correct flag set
                } else {
                    // result is an empty set
                    self.clear();
                }
            }
            (Representation::Sparse, Representation::Inline | Representation::Alloc) => {
                let flag = unsafe { self.get_sparse_data_unchecked() };
                let target_elem = flag / BITS;
                let target_shift = flag % BITS;

                let other_elem = other.data().get(target_elem).copied().unwrap_or_default();
                let other_flag = ((other_elem >> target_shift) & 1) * flag;

                if sparse_cond(flag, other_flag) {
                    // result is still sparse and lhs already has the correct flag set
                } else {
                    // result is an empty set
                    self.clear();
                }
            }
            (Representation::Inline | Representation::Alloc, Representation::Sparse) => {
                let other_flag = unsafe { other.get_sparse_data_unchecked() };
                let target_elem = other_flag / BITS;
                let target_shift = other_flag % BITS;

                let elem = self.data().get(target_elem).copied().unwrap_or_default();
                let flag = ((elem >> target_shift) & 1) * other_flag;

                match (sparse_cond(flag, other_flag), flag == other_flag) {
                    (true, true) => self.clone_from(other), // result is sparse and lhs needs to be updated
                    (false, false) => self.clear(),         // result is an empty set
                    (true, false) | (false, true) => {
                        // result is not sparse, rhs_flag bit needs to be unset in lhs
                        self.and_not_assign(&(Self::new_inline(1) << other_flag));
                    }
                }
            }
            (Representation::Inline, Representation::Inline | Representation::Alloc) => {
                let mut lhs = unsafe { self.get_inline_data_unchecked() };
                let rhs = other.data()[0];
                op(&mut lhs, rhs);
                unsafe { self.write_inline_data_unchecked(lhs) }
            }
            (Representation::Alloc, Representation::Alloc) => {
                let lhs = unsafe { self.as_slice_mut_unchecked() };
                let rhs = unsafe { other.as_slice_unchecked() };

                // in case lhs > rhs we need to have extra elements
                let rhs_iter = rhs.iter().chain(iter::repeat(&0));

                for (lhs, rhs) in lhs.iter_mut().zip(rhs_iter) {
                    op(lhs, *rhs);
                }
            }
            (Representation::Alloc, Representation::Inline) => {
                let lhs = unsafe { self.as_slice_mut_unchecked() };
                let rhs = unsafe { other.get_inline_data_unchecked() };

                lhs.iter_mut().enumerate().for_each(|(idx, lhs)| {
                    op(lhs, rhs.checked_shr((idx * BITS) as u32).unwrap_or(0));
                });
            }
        }
    }

    fn extending_bitop_assign(
        &mut self,
        other: &Self,
        mut op: impl FnMut(&mut usize, usize),
        sparse_cond: impl Fn(usize, usize) -> bool,
    ) {
        match (self.representation(), other.representation()) {
            (Representation::Sparse, Representation::Sparse) => {
                let flag = unsafe { self.get_sparse_data_unchecked() };
                let other_flag = unsafe { other.get_sparse_data_unchecked() };

                if sparse_cond(flag, other_flag) {
                    if flag == other_flag {
                        // result is still sparse and lhs is already correct
                    } else {
                        // result is not sparse, must contain both flags
                        let mut res = self.normalize();
                        res.extending_bitop_assign(&other.normalize(), op, sparse_cond);
                        *self = res;
                    }
                } else {
                    // result is an empty set
                    self.clear();
                }
            }
            (Representation::Sparse, Representation::Inline | Representation::Alloc) => {
                let mut normalized = self.normalize();
                normalized.extending_bitop_assign(other, op, sparse_cond);
                *self = normalized;
            }
            (Representation::Inline | Representation::Alloc, Representation::Sparse) => {
                self.extending_bitop_assign(&other.normalize(), op, sparse_cond);
            }
            (Representation::Inline, Representation::Inline) => {
                let mut lhs = unsafe { self.get_inline_data_unchecked() };
                let rhs = unsafe { other.get_inline_data_unchecked() };
                op(&mut lhs, rhs);
                unsafe { self.write_inline_data_unchecked(lhs) };
            }
            (Representation::Inline | Representation::Alloc, Representation::Alloc) => {
                let rhs_hb = other.len();
                let lhs_hb = self.len();
                if rhs_hb > lhs_hb {
                    self.reserve(rhs_hb - lhs_hb);
                }

                let lhs = unsafe { self.as_slice_mut_unchecked() };
                let rhs = unsafe { other.as_slice_unchecked() };

                // in case lhs > rhs we need to have extra elements
                let rhs_iter = rhs.iter().chain(iter::repeat(&0));

                for (lhs, rhs) in lhs.iter_mut().zip(rhs_iter) {
                    op(lhs, *rhs);
                }
            }
            (Representation::Alloc, Representation::Inline) => {
                let lhs = unsafe { self.as_slice_mut_unchecked() };
                let rhs = unsafe { other.get_inline_data_unchecked() };

                if let Some(lhs) = lhs.iter_mut().next() {
                    op(lhs, rhs);
                }
            }
        }
    }
}

macro_rules! impl_bitop {
    ($($OP:ident :: $op:ident, $OPA:ident :: $opa:ident, $sparse_cond:expr, $body_macro:path;)+) => {$(
        impl $OP<Self> for SmolBitSet {
            type Output = Self;

            #[inline]
            fn $op(self, rhs: Self) -> Self {
                let mut lhs = self;
                lhs.$opa(rhs);
                lhs
            }
        }

        impl $OPA<Self> for SmolBitSet {
            #[inline]
            fn $opa(&mut self, rhs: Self) {
                self.$opa(&rhs);
            }
        }

        impl $OP<&Self> for SmolBitSet {
            type Output = Self;

            #[inline]
            fn $op(self, rhs: &Self) -> Self {
                let mut lhs = self;
                lhs.$opa(rhs);
                lhs
            }
        }

        impl $OPA<&Self> for SmolBitSet {
            fn $opa(&mut self, rhs: &Self) {
                $body_macro(self, rhs, |lhs, rhs| lhs.$opa(rhs), $sparse_cond);
            }
        }
    )*};
}

impl_bitop! {
    BitOr::bitor, BitOrAssign::bitor_assign, |_, _| true, SmolBitSet::extending_bitop_assign;
    BitAnd::bitand, BitAndAssign::bitand_assign, |lhs, rhs| lhs == rhs, SmolBitSet::shortening_bitop_assign;
    BitXor::bitxor, BitXorAssign::bitxor_assign, |lhs, rhs| lhs != rhs, SmolBitSet::extending_bitop_assign;
}

macro_rules! impl_bitop_prim {
    ($($OP:ident :: $op:ident, $OPA:ident :: $opa:ident, $t:ty;)+) => {$(
        impl $OP<$t> for SmolBitSet {
            type Output = Self;

            #[inline]
            fn $op(self, rhs: $t) -> Self {
                let mut lhs = self;
                lhs.$opa(rhs);
                lhs
            }
        }

        impl $OPA<$t> for SmolBitSet {
            #[inline]
            fn $opa(&mut self, rhs: $t) {
                self.$opa(Self::from(rhs))
            }
        }

        impl $OP<&$t> for SmolBitSet {
            type Output = Self;

            #[inline]
            fn $op(self, rhs: &$t) -> Self {
                self.$op(*rhs)
            }
        }

        impl $OPA<&$t> for SmolBitSet {
            #[inline]
            fn $opa(&mut self, rhs: &$t) {
                self.$opa(*rhs)
            }
        }
    )*};
    ($($t:ty),+) => {$(
        impl_bitop_prim!{
            BitOr::bitor, BitOrAssign::bitor_assign, $t;
            BitAnd::bitand, BitAndAssign::bitand_assign, $t;
            BitXor::bitxor, BitXorAssign::bitxor_assign, $t;
        }
    )*};
}

impl_bitop_prim!(u8, u16, u32, u64, u128, usize);

impl SmolBitSet {
    /// Unsets all bits set on the `rhs` [`SmolBitSet`].
    ///
    /// This is equivalent to `self & !rhs` with integers.
    #[must_use]
    pub fn and_not(mut self, rhs: &Self) -> Self {
        self.and_not_assign(rhs);
        self
    }

    /// Unsets all bits set on the `rhs` [`SmolBitSet`].
    ///
    /// This is equivalent to `*self &= !rhs` with integers.
    pub fn and_not_assign(&mut self, rhs: &Self) {
        self.shortening_bitop_assign(rhs, |lhs, rhs| *lhs &= !rhs, |lhs, rhs| lhs != rhs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "std"))]
    use extern_alloc::vec::Vec;

    /// Helper extension trait to simplify some tests.
    trait AndNot {
        fn and_not(self, rhs: Self) -> Self;
    }

    impl AndNot for usize {
        fn and_not(self, rhs: Self) -> Self {
            self & !rhs
        }
    }

    fn zero_pad_to(mut vec: Vec<usize>, len: usize) -> Vec<usize> {
        while vec.len() < len {
            vec.push(0);
        }
        vec
    }

    mod normal {
        use super::*;

        mod inline {
            use super::*;

            macro_rules! test_inline_bitops {
                ($($name:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        let a = SmolBitSet::from($a);
                        let b = SmolBitSet::from($b);
                        assert!(a.is_inline());
                        assert!(b.is_inline());

                        let res = a.$name(&b);
                        assert!(res.is_inline());

                        let d = unsafe { res.get_inline_data_unchecked() };
                        assert_eq!(d, ($a as usize).$name($b as usize));
                    }
                )*}
            }

            test_inline_bitops! {
                bitor, 0xF0F0u16, 0x0F0Fu16,
                bitand, 0xFEFEu16, 0xAFFEu16,
                bitxor, 0x3A52u16, 0xAAE8u16,
                and_not, 0xFEFEu16, 0xAFFEu16
            }
        }

        mod slice {
            use super::*;

            macro_rules! test_slice_bitops {
                ($($name:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        let a = SmolBitSet::from($a);
                        let b = SmolBitSet::from($b);
                        assert_eq!(a.capacity(), 64);
                        assert_eq!(b.capacity(), 64);

                        let res = a.$name(&b);
                        assert_eq!(res.capacity(), 64);
                        assert_eq!(res.data().as_ref(), [($a as usize).$name($b as usize)]);
                    }
                )*}
            }

            test_slice_bitops! {
                bitor, 0xC0FF_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitand, 0xC0FF_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitxor, 0xC0FF_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                and_not, 0xC0FF_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64
            }
        }

        mod mixed {
            use super::*;

            macro_rules! test_mixed_bitops {
                ($($name:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        let a = SmolBitSet::from($a);
                        let b = SmolBitSet::from($b);
                        assert!(a.is_inline());
                        assert_eq!(b.capacity(), 64);

                        let res1 = a.clone().$name(&b);
                        assert_eq!(res1.data().as_ref(), [($a as usize).$name($b as usize)]);

                        let res2 = b.$name(&a);
                        assert_eq!(res2, res1);
                    }
                )*}
            }

            test_mixed_bitops! {
                bitor, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitand, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitxor, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64
            }

            // `a.and_not(b) != b.and_not(a)`
            #[test]
            fn and_not() {
                const A: u64 = 0x0ABC_EE00_1337_BEEFu64;
                const B: u64 = 0xF00D_BEEF_0420_BEEFu64;

                let a = SmolBitSet::from(A);
                let b = SmolBitSet::from(B);
                assert!(a.is_inline());
                assert_eq!(b.capacity(), 64);

                let res1 = a.and_not(&b);
                assert_eq!(res1.data().as_ref(), [(A as usize).and_not(B as usize)]);
            }
        }

        mod rhs_smaller {
            use super::*;

            macro_rules! test_rhs_smaller_bitops {
                ($($name:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        let mut lhs = SmolBitSet::from($a);
                        let rhs = SmolBitSet::from($b);
                        lhs <<= 32u8;
                        let lhs_capacity = lhs.capacity();
                        assert!(lhs_capacity > rhs.capacity());

                        let res = lhs.$name(&rhs);
                        assert_eq!(
                            zero_pad_to(res.data().into_owned(), 2),
                            [
                                ((($a << 32) as usize).$name($b as usize)),
                                ((($a >> 32) as usize).$name(0 as usize))
                            ]
                        );
                    }
                )*};
            }

            test_rhs_smaller_bitops! {
                bitor, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitand, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitxor, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                and_not, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64
            }
        }

        mod rhs_larger {
            use super::*;

            macro_rules! test_rhs_larger_bitops {
                ($($name:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        let lhs = SmolBitSet::from($a);
                        let mut rhs = SmolBitSet::from($b);
                        rhs <<= 32u8;
                        let rhs_capacity = rhs.capacity();
                        assert!(rhs_capacity > lhs.capacity());

                        let res = lhs.$name(&rhs);
                        assert_eq!(
                            zero_pad_to(res.data().into_owned(), 2),
                            [
                                (($a as usize).$name(($b << 32) as usize)),
                                ((0 as usize).$name(($b >> 32) as usize))
                            ]
                        );
                    }
                )*};
            }

            test_rhs_larger_bitops! {
                bitor, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitand, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                bitxor, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64,
                and_not, 0x0ABC_EE00_1337_BEEFu64, 0xF00D_BEEF_0420_BEEFu64
            }
        }
    }

    mod sparse {
        use super::*;

        mod inline {
            use super::*;

            macro_rules! test_sparse_inline_bitops {
                ($($name:ident, $op:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        let a = SmolBitSet::flag($a);
                        let b = SmolBitSet::flag($b);
                        assert!(a.is_inline());
                        assert!(b.is_inline());
                        assert!(a.is_sparse());
                        assert!(b.is_sparse());

                        let res = a.$op(&b);
                        assert!(res.is_inline());

                        let normal_res = (1usize << $a as usize).$op(1usize << $b as usize);
                        if res.is_sparse() {
                            let res_flag = unsafe { res.get_sparse_data_unchecked() };
                            assert_eq!(res_flag, normal_res.trailing_zeros() as usize);
                        } else {
                            let d = unsafe { res.get_inline_data_unchecked() };
                            assert_eq!(d, normal_res);
                        }
                    }
                )*}
            }

            test_sparse_inline_bitops! {
                bitor_eq, bitor, 10, 10,
                bitor_ne, bitor, 14, 18,

                bitand_eq, bitand, 22, 22,
                bitand_ne, bitand, 12, 22,

                bitxor_eq, bitxor, 5, 5,
                bitxor_ne, bitxor, 7, 24,

                and_not_eq, and_not, 13, 13,
                and_not_ne, and_not, 17, 19
            }
        }
    }

    mod mixed {
        use super::*;

        mod inline {
            use super::*;

            macro_rules! test_mixed_inline_bitops {
                ($($name:ident, $op:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        fn inner(a: SmolBitSet, b: SmolBitSet, a_u: usize, b_u: usize) {
                            let res = a.$op(&b);
                            assert!(res.is_inline());

                            let normal_res = a_u.$op(b_u);

                            if res.is_sparse() {
                                let res_flag = unsafe { res.get_sparse_data_unchecked() };
                                assert_eq!(normal_res.count_ones(), 1);
                                assert_eq!(res_flag, normal_res.trailing_zeros() as usize);
                            } else {
                                let d = unsafe { res.get_inline_data_unchecked() };
                                assert_eq!(d, normal_res);
                            }
                        }

                        let a = SmolBitSet::flag($a);
                        let b = SmolBitSet::from($b);
                        assert!(a.is_inline());
                        assert!(a.is_sparse());
                        assert!(b.is_inline());
                        assert!(!b.is_sparse());

                        let a_u = 1usize << $a as usize;
                        let b_u = $b as usize;

                        inner(a.clone(), b.clone(), a_u, b_u);
                        inner(b, a, b_u, a_u);
                    }
                )*}
            }

            test_mixed_inline_bitops! {
                bitor_eq, bitor, 10, 0xBEEFu16,
                bitor_ne, bitor, 10, 0xF00Du16,

                bitand_eq, bitand, 10, 0xBEEFu16,
                bitand_ne, bitand, 10, 0xF00Du16,

                bitxor_eq, bitxor, 10, 0xBEEFu16,
                bitxor_ne, bitxor, 10, 0xF00Du16,

                and_not_eq, and_not, 10, 0xBEEFu16,
                and_not_ne, and_not, 10, 0xF00Du16
            }
        }

        mod sparse_inline_normal_heap {
            use super::*;

            macro_rules! test_mixed_sparse_inline_normal_heap_bitops {
                ($($name:ident, $op:ident, $a:expr, $b:expr),+) => {$(
                    #[test]
                    fn $name() {
                        fn inner(a: SmolBitSet, b: SmolBitSet, a_u: usize, b_u: usize, shift: usize) {
                            let res = a.$op(&b);

                            let normal_res = a_u.$op(b_u);
                            let shifted_res = SmolBitSet::from(normal_res) << shift;
                            assert_eq!(res, shifted_res);
                        }

                        let shift = 120;
                        let a: SmolBitSet = SmolBitSet::flag($a) << shift;
                        let b: SmolBitSet = SmolBitSet::from($b) << shift;
                        assert!(a.is_inline());
                        assert!(a.is_sparse());
                        assert!(!b.is_inline());
                        assert!(!b.is_sparse());

                        let a_u = 1usize << $a as usize;
                        let b_u = $b as usize;

                        inner(a.clone(), b.clone(), a_u, b_u, shift);
                        inner(b, a, b_u, a_u, shift);
                    }
                )*}
            }

            test_mixed_sparse_inline_normal_heap_bitops! {
                bitor_eq, bitor, 10, 0xBEEFu16,
                bitor_ne, bitor, 10, 0xF00Du16,

                bitand_eq, bitand, 10, 0xBEEFu16,
                bitand_ne, bitand, 10, 0xF00Du16,

                bitxor_eq, bitxor, 10, 0xBEEFu16,
                bitxor_ne, bitxor, 10, 0xF00Du16,

                and_not_eq, and_not, 10, 0xBEEFu16,
                and_not_ne, and_not, 10, 0xF00Du16
            }
        }
    }
}
