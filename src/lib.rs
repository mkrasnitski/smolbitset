//! A crate for dynamically sized bitsets with memory usage optimizations.
//!
//! Supports 64 and 32 bit targets and integrates with `serde` and `typesize`. Also supports
//! `no_std` environments by disabling the `std` feature. The `no_std` environment must support
//! [`alloc`].
//!
//! Constructing a [`SmolBitSet`] in a `const` context is supported in the following ways:
//! 1. If the value has multiple set bits, call [`SmolBitSet::new_inline`].
//! 2. If the value has only a single set bit (i.e. it represents a flag), [`SmolBitSet::flag`] is
//!    recommended.
//!
//! ## Memory usage
//!
//! Bitsets of small enough size are stored inline using a single `usize`, and otherwise are
//! allocated on the heap.
//!
//! | Target Pointer Size | [`size_of::<SmolBitSet>`] | Inline Capacity | Max Heap Capacity |
//! |--------------------:|--------------------------:|----------------:|------------------:|
//! | 32 bits             | 4 bytes                   | 30 bits         | 2^36 bits         |
//! | 64 bits             | 8 bytes                   | 62 bits         | 2^68 bits         |
//!
//! Furthermore, [`SmolBitSet`] has a niche optimization so [`Option<SmolBitSet>`] has the same size
//! as [`SmolBitSet`].
//!
//! ## Limitations
//!
//! * [`SmolBitSet`] does not implement [`Copy`].
//! * Implementing [`core::ops::Not`] is also not possible (or rather complex).\
//!   Related alternative methods are provided via [`SmolBitSet::and_not`] and [`SmolBitSet::and_not_assign`].
//!
//! # Example
//!
//! ```
//! use smolbitset::SmolBitSet;
//!
//! let mut sbs = SmolBitSet::empty();
//!
//! sbs |= 1u32 << 5;
//! sbs >>= 5u8;
//! assert_eq!(sbs, SmolBitSet::from(1u64));
//!
//! sbs |= !1u64;
//! assert_eq!(sbs, SmolBitSet::from(u64::MAX));
//!
//! sbs <<= 64u16;
//! assert_eq!(sbs, SmolBitSet::from_bits(&(64..128).collect::<Box<[_]>>()))
//! ```
//!
//! # Minimum Supported Rust Version
//!
//! Currently this crate supports an MSRV of Rust 1.89.0, and increasing the MSRV is considered a
//! breaking change.

#![doc(html_root_url = "https://docs.rs/smolbitset/*")]
#![allow(dead_code)]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc as extern_alloc;
#[cfg(not(feature = "std"))]
use extern_alloc::{
    alloc::{self, Layout, handle_alloc_error},
    borrow::Cow,
    vec,
};

#[cfg(feature = "std")]
use std::alloc::{self, Layout, handle_alloc_error};
#[cfg(feature = "std")]
use std::borrow::Cow;

use core::num::NonZero;
use core::ptr::NonNull;
use core::slice;

/// Returns the index of the most significant bit set to 1 in the given data.
macro_rules! highest_set_bit {
    ($t:ty, $val:expr) => {
        core::num::NonZero::new($val).map(|v| (<$t>::BITS - 1 - v.leading_zeros()) as usize)
    };
}

mod bitop;
mod cmp;
mod fmt;
mod from;
mod hash;
mod shifts;

#[cfg(feature = "serde")]
mod serde;

#[cfg(feature = "typesize")]
mod typesize;

/// How many bits are used for other purposes in the pointer which also determines
/// the required alignment since we use the least significant bits for this header information.
///
/// Bit 0 is used to determine whether the data is stored inline or on the heap (0 = heap, 1 = inline).\
/// Bit 1 is used to determine the mode in which the bits are represented (0 = normal, 1 = sparse).\
/// Sparse mode is an optimization for bitsets with very few bits set to 1.
/// In this mode the set bits are stored as a list of indices instead.\
/// This also allows us to create a [`SmolBitSet`] in const contexts for bit values that would not fit
/// in the normal inline representation.
const HEADER_SIZE: u32 = 2;

enum Representation {
    NormalInline = 0b01,
    SparseInline = 0b11,
    NormalHeap = 0b00,
}

const BITS: usize = usize::BITS as usize;
const MAX_INLINE_BITS: usize = (usize::BITS - HEADER_SIZE) as usize;
const MAX_INLINE_VAL: usize = usize::MAX >> HEADER_SIZE;

/// A dynamically sized bitset with memory usage optimizations.
#[repr(transparent)]
pub struct SmolBitSet {
    ptr: NonNull<usize>,
}

impl SmolBitSet {
    /// Constructs a new, empty [`SmolBitSet`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// let mut sbs = SmolBitSet::empty();
    /// ```
    #[must_use]
    #[inline]
    pub const fn empty() -> Self {
        let ptr = NonNull::without_provenance(core::num::NonZero::<usize>::MIN);

        Self { ptr }
    }

    /// Constructs a new [`SmolBitSet`] from the provided `val` without any heap allocations.
    ///
    /// # Panics
    ///
    /// Panics if any of the 2 most significant bits in `val` is 1.
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// const sbs: SmolBitSet = SmolBitSet::new_inline(1234);
    /// assert_eq!(sbs, SmolBitSet::from(1234u16));
    /// ```
    #[must_use]
    pub const fn new_inline(val: usize) -> Self {
        assert!(
            val <= MAX_INLINE_VAL,
            "val too large for a non allocating SmolBitSet"
        );

        let mut res = Self::empty();
        unsafe {
            res.write_inline_data_unchecked(val);
        }

        res
    }

    /// Constructs a new [`SmolBitSet`] from the provided `bit` index without any heap allocations.
    ///
    /// The returned bitset will represent the value `1 << bit`.
    ///
    /// # Panics
    ///
    /// Panics if any of the 2 most significant bits in `bit` is 1.
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// const sbs: SmolBitSet = SmolBitSet::flag(1234);
    /// assert_eq!(sbs, SmolBitSet::from(1u64) << 1234);
    /// ```
    #[must_use]
    pub const fn flag(bit: usize) -> Self {
        assert!(
            bit <= MAX_INLINE_VAL,
            "bit index out of range for a non allocating sparse SmolBitSet"
        );

        let mut res = Self::empty();
        unsafe {
            res.write_sparse_data_unchecked(bit);
        }

        res
    }

    /// Constructs a new [`SmolBitSet`] from the provided array of bit indices without any heap allocations.
    ///
    /// # Panics
    ///
    /// Panics if any bit index in `bits` is larger than or equal to <code>[usize::BITS] - 2</code>.
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// const sbs: SmolBitSet = SmolBitSet::from_bits_inline([0, 4, 1, 6]);
    /// assert_eq!(sbs, SmolBitSet::from(0b0101_0011u8));
    /// ```
    ///
    /// ```should_panic
    /// # use smolbitset::SmolBitSet;
    /// // this panics since 62 is outside of the range
    /// // a SmolBitSet can hold without incurring a heap allocation
    /// let sbs = SmolBitSet::from_bits_inline([62]);
    /// ```
    ///
    /// ```compile_fail
    /// # use smolbitset::SmolBitSet;
    /// // this fails to compile since the const evaluation
    /// // panics for the same reason as above
    /// const sbs: SmolBitSet = SmolBitSet::from_bits_inline([62]);
    /// ```
    #[must_use]
    pub const fn from_bits_inline<const N: usize>(bits: [usize; N]) -> Self {
        let mut res = 0;
        let mut i = 0;

        while i < N {
            let b = bits[i];
            assert!(
                b < MAX_INLINE_BITS,
                "bit index out of range for a non allocating SmolBitSet"
            );

            res |= 1 << b;
            i += 1;
        }

        Self::new_inline(res)
    }

    /// Creates a new [`SmolBitSet`] from the provided slice of bit indices.
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// // bit indices can be in any order
    /// let sbs = SmolBitSet::from_bits(&[0, 6, 4, 3]);
    /// assert_eq!(sbs, SmolBitSet::from(0b0101_1001u8));
    ///
    /// let sbs = SmolBitSet::from_bits(&[63]);
    /// assert_eq!(sbs, SmolBitSet::from(1u64 << 63));
    /// ```
    #[must_use]
    pub fn from_bits(bits: &[usize]) -> Self {
        // TODO: check if sparse representation would be more efficient for the given bit indices

        let mut res = Self::empty();

        let Some(b) = bits.iter().copied().max() else {
            return res;
        };

        res.ensure_capacity(b + 1);

        if res.is_inline() {
            let mut data = 0;

            for &bit in bits {
                data |= 1 << bit;
            }

            unsafe { res.write_inline_data_unchecked(data) }
        } else {
            let data = unsafe { res.as_slice_mut_unchecked() };

            for &bit in bits {
                let s = bit % BITS;
                let b = bit / BITS;
                data[b] |= 1 << s;
            }
        }

        res
    }

    #[inline]
    fn representation(&self) -> Representation {
        match self.ptr.addr().get() & 0b11 {
            0b00 => Representation::NormalHeap,
            0b01 => Representation::NormalInline,
            0b11 => Representation::SparseInline,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn is_inline(&self) -> bool {
        matches!(
            self.representation(),
            Representation::NormalInline | Representation::SparseInline
        )
    }

    #[inline]
    unsafe fn get_inline_data_unchecked(&self) -> usize {
        self.ptr.addr().get() >> HEADER_SIZE
    }

    #[inline]
    const unsafe fn write_inline_data_unchecked(&mut self, data: usize) {
        debug_assert!(data <= MAX_INLINE_VAL);

        let addr = unsafe { NonZero::new_unchecked((data << HEADER_SIZE) | 0b01) };
        self.ptr = NonNull::without_provenance(addr);
    }

    #[inline]
    fn is_sparse(&self) -> bool {
        matches!(self.representation(), Representation::SparseInline)
    }

    unsafe fn get_sparse_data_unchecked(&self) -> usize {
        self.ptr.addr().get() >> HEADER_SIZE
    }

    #[inline]
    const unsafe fn write_sparse_data_unchecked(&mut self, data: usize) {
        debug_assert!(data <= MAX_INLINE_VAL);

        let addr = unsafe { NonZero::new_unchecked((data << HEADER_SIZE) | 0b11) };
        self.ptr = NonNull::without_provenance(addr);
    }

    #[inline]
    fn len(&self) -> usize {
        if self.is_inline() {
            return 0;
        }

        unsafe { self.len_unchecked() }
    }

    #[inline]
    const unsafe fn len_unchecked(&self) -> usize {
        unsafe { *self.ptr.as_ptr() }
    }

    #[inline]
    const unsafe fn data_ptr_unchecked(&self) -> *mut usize {
        unsafe { self.ptr.as_ptr().add(1) }
    }

    /// Returns the underlying data of the [`SmolBitSet`] split into individual [`usize`] blocks.
    #[must_use]
    pub fn data(&self) -> Cow<'_, [usize]> {
        if self.is_inline() {
            let data = unsafe { self.get_inline_data_unchecked() };
            vec![data].into()
        } else {
            unsafe { self.as_slice_unchecked().into() }
        }
    }

    #[inline]
    const unsafe fn as_slice_unchecked(&self) -> &[usize] {
        unsafe { slice::from_raw_parts(self.data_ptr_unchecked(), self.len_unchecked()) }
    }

    #[inline]
    const unsafe fn as_slice_mut_unchecked(&mut self) -> &mut [usize] {
        unsafe { slice::from_raw_parts_mut(self.data_ptr_unchecked(), self.len_unchecked()) }
    }

    fn as_normal(&self) -> Self {
        if !self.is_sparse() {
            return self.clone();
        }

        debug_assert!(
            self.is_inline(),
            "sparse heap representation is not implemented yet"
        );

        let flag = unsafe { self.get_sparse_data_unchecked() };
        Self::new_inline(1) << flag
    }

    #[inline]
    fn spill(&mut self, capacity: usize) {
        if !self.is_inline() {
            return;
        }

        let len = capacity.div_ceil(BITS);

        let layout = create_layout(len);
        let ptr = unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            alloc::alloc(layout).cast::<usize>()
        };
        if ptr.is_null() {
            handle_alloc_error(layout)
        }

        unsafe {
            *ptr = len; // store the length in the first element
            let old = self.get_inline_data_unchecked();
            *ptr.add(1) = old;

            slice::from_raw_parts_mut(ptr.add(2), len - 1).fill(0);
        };

        self.ptr = unsafe { NonNull::new_unchecked(ptr) };
    }

    #[inline]
    fn ensure_capacity(&mut self, capacity: usize) {
        if self.is_inline() {
            if capacity > MAX_INLINE_BITS {
                self.spill(capacity)
            }
        } else {
            let len = unsafe { self.len_unchecked() };
            if capacity >= (BITS * len) {
                unsafe { self.grow(len, capacity) }
            }
        }
    }

    unsafe fn grow(&mut self, len: usize, capacity: usize) {
        // we need to grow our slice allocation
        let new_len = capacity.div_ceil(BITS);
        debug_assert!(new_len >= len);

        let layout = create_layout(len);
        let new_layout = create_layout(new_len);
        let new_ptr = unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            alloc::realloc(self.ptr.as_ptr().cast(), layout, new_layout.size()).cast::<usize>()
        };
        if new_ptr.is_null() {
            handle_alloc_error(new_layout)
        }

        unsafe {
            // initializing newly allocated memory to zero
            slice::from_raw_parts_mut(new_ptr.add(1 + len), new_len - len).fill(0);

            // update the new length in the first element
            *new_ptr = new_len;
        }
        self.ptr = unsafe { NonNull::new_unchecked(new_ptr) };
    }

    /// Returns the index of the most significant bit set to 1.
    #[inline]
    fn highest_set_bit(&self) -> Option<usize> {
        match self.representation() {
            Representation::NormalInline => {
                let data = unsafe { self.get_inline_data_unchecked() };
                highest_set_bit!(usize, data)
            }
            Representation::NormalHeap => {
                let data = unsafe { self.as_slice_unchecked() };
                for (idx, &data) in data.iter().enumerate().rev() {
                    if let Some(h) = highest_set_bit!(usize, data) {
                        return Some((idx * BITS) + h);
                    }
                }

                None
            }
            Representation::SparseInline => unsafe { Some(self.get_sparse_data_unchecked()) },
        }
    }
}

impl Drop for SmolBitSet {
    #[inline]
    fn drop(&mut self) {
        if self.is_inline() {
            return;
        }

        unsafe {
            let layout = create_layout(self.len_unchecked());
            alloc::dealloc(self.ptr.cast::<u8>().as_ptr(), layout);
        }
    }
}

unsafe impl Send for SmolBitSet {}
unsafe impl Sync for SmolBitSet {}

impl Default for SmolBitSet {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl Clone for SmolBitSet {
    fn clone(&self) -> Self {
        if self.is_inline() {
            return Self { ptr: self.ptr };
        }

        let src = unsafe { self.as_slice_unchecked() };
        let len = src.len();
        let layout = create_layout(len);
        let ptr = unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            alloc::alloc_zeroed(layout).cast::<usize>()
        };
        if ptr.is_null() {
            handle_alloc_error(layout)
        }

        let new_data = unsafe {
            *ptr = len; // store the length in the first element
            slice::from_raw_parts_mut(ptr.add(1), len)
        };
        new_data.copy_from_slice(src);

        let ptr = unsafe { NonNull::new_unchecked(ptr) };
        Self { ptr }
    }
}

#[inline]
fn create_layout(len: usize) -> Layout {
    assert!(
        len.checked_mul(size_of::<usize>()).is_some(),
        "overflow error in SmolBitSet slice"
    );

    if let Ok(layout) = Layout::array::<usize>(len + 1)
        // Ensure the address of our allocation will have all 0s in the header bits so that
        // `SmolBitSet::representation` will return `Representation::NormalHeap`. As it stands,
        // `align_of::<usize>()` is the same as `size_of::<usize>` and this is a no-op, but
        // better to be explicit.
        && let Ok(layout) = layout.align_to(1 << HEADER_SIZE)
    {
        return layout;
    }

    panic!("invalid layout when allocating SmolBitSet");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[cfg(not(feature = "std"))]
    use extern_alloc::string::{String, ToString};

    #[test]
    fn send() {
        fn assert_send<T: Send>() {}
        assert_send::<SmolBitSet>();
    }

    #[test]
    fn sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<SmolBitSet>();
    }

    #[test]
    fn check_highest_set_bit() {
        let mut t: u64 = 0;
        assert_eq!(highest_set_bit!(u64, t), None);

        t = 1;
        assert_eq!(highest_set_bit!(u64, t), Some(0));

        t = 1 << 3;
        assert_eq!(highest_set_bit!(u64, t), Some(3));

        t = 1 << 31;
        assert_eq!(highest_set_bit!(u64, t), Some(31));

        t = 0b10101;
        assert_eq!(highest_set_bit!(u64, t), Some(4));

        t = u64::MAX;
        assert_eq!(highest_set_bit!(u64, t), Some(63));
    }

    #[test]
    fn ensure_capacity() {
        let mut t = SmolBitSet::empty();
        assert!(t.is_inline());

        t.ensure_capacity(0);
        assert!(t.is_inline());

        t.ensure_capacity(32);
        assert!(t.is_inline());

        let max_inline = MAX_INLINE_BITS;
        t.ensure_capacity(max_inline);
        assert!(t.is_inline());

        t.ensure_capacity(max_inline + 1);
        assert!(!t.is_inline());
        assert_eq!(t.len(), 1);

        t.ensure_capacity(65);
        assert!(!t.is_inline());
        assert_eq!(t.len(), 2);

        t.ensure_capacity(64 * 40);
        assert!(!t.is_inline());
        assert_eq!(t.len(), 40);
    }

    #[test]
    fn set_get_inline() {
        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        unsafe {
            let d = sbs.get_inline_data_unchecked();
            assert_eq!(d, 0);

            sbs.write_inline_data_unchecked(0b1010);
            assert!(sbs.is_inline());

            let d = sbs.get_inline_data_unchecked();
            assert_eq!(d, 0b1010);
        }
    }

    #[test]
    fn set_get_slice() {
        let a = SmolBitSet::from(0xC5C5_BEEF_0000_1234u64);
        assert!(!a.is_inline());
        assert_eq!(a.len(), 1);

        let d1 = a.data();
        assert_eq!(d1.len(), 1);
        assert_eq!(d1.as_ref(), [0xC5C5_BEEF_0000_1234]);

        let mut b = a.clone();
        let d2 = b.data();
        assert_eq!(d2.len(), 1);
        assert_eq!(d2, d1.as_ref());

        b &= 0u64;
        b |= 0xC0FF_EE00_DEAD_BEEFu64;

        let d3 = b.data();
        assert_eq!(d3.len(), 1);
        assert_eq!(d3.as_ref(), [0xC0FF_EE00_DEAD_BEEF]);
    }

    #[test]
    fn spill() {
        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.spill(30);
        assert!(!sbs.is_inline());
        // expecting 1 since the inline data can hold 63 bits already
        // and spill will always allocate to at least store the inline data
        assert_eq!(sbs.len(), 1);

        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.spill(55);
        assert!(!sbs.is_inline());
        assert_eq!(sbs.len(), 1);

        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.spill(64);
        assert!(!sbs.is_inline());
        assert_eq!(sbs.len(), 1);

        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.spill(65);
        assert!(!sbs.is_inline());
        assert_eq!(sbs.len(), 2);
    }

    #[test]
    fn deserialize() {
        let sbs = SmolBitSet::try_from(String::from("1337")).unwrap();
        assert!(sbs.is_inline());
        assert_eq!(unsafe { sbs.get_inline_data_unchecked() }, 1337);

        // A5A5 1337 0000 C0FF EE00 BEEF 0000 A5A5
        let sbs =
            SmolBitSet::try_from(String::from("220179738009501684669546686565819917733")).unwrap();
        assert!(!sbs.is_inline());
        assert_eq!(
            sbs.data().as_ref(),
            [0xEE00_BEEF_0000_A5A5, 0xA5A5_1337_0000_C0FF]
        );
    }

    #[test]
    fn serialize() {
        let sbs = SmolBitSet::from(1337u32);
        assert_eq!(sbs.to_string(), "1337");

        // A5A5 1337 0000 C0FF EE00 BEEF 0000 A5A5
        let mut sbs = SmolBitSet::from(0xA5A5_1337_0000_C0FFu64);
        sbs <<= 64u8;
        sbs |= 0xEE00_BEEF_0000_A5A5u64;
        assert_eq!(sbs.to_string(), "220179738009501684669546686565819917733");
    }

    mod clone {
        use super::*;

        #[test]
        fn inline() {
            let val = 0xC0FE_FE00u32;
            let a = SmolBitSet::from(val);
            #[allow(clippy::redundant_clone)]
            let b = a.clone();

            assert!(a.is_inline());
            assert!(b.is_inline());

            let a_data = unsafe { a.get_inline_data_unchecked() };
            let b_data = unsafe { b.get_inline_data_unchecked() };
            assert_eq!(a_data, b_data);
        }

        #[test]
        fn slice() {
            let val = 0xFFEE_00AA_1337_0420u64;
            let a = SmolBitSet::from(val);
            #[allow(clippy::redundant_clone)]
            let b = a.clone();

            assert!(!a.is_inline());
            assert!(!b.is_inline());

            let a_data = a.data();
            let b_data = b.data();
            assert_eq!(a_data.len(), b_data.len());
            assert_eq!(a_data, b_data);
            assert_eq!(a_data.as_ref(), [0xFFEE_00AA_1337_0420]);
        }
    }
}
