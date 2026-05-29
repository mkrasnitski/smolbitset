//! A crate for dynamically sized bitsets with memory usage optimizations.
//!
//! Supports 64 and 32 bit targets and integrates with `serde` and `typesize`. Also supports
//! `no_std` environments by disabling the `std` feature. The `no_std` environment must support
//! [`alloc`].
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
//! # Const support
//!
//! Constructing a [`SmolBitSet`] in a `const` context is supported in the following ways:
//! 1. If the value has multiple set bits, call [`SmolBitSet::new_inline`].
//! 2. If the value has only a single set bit (i.e. it represents a flag), [`SmolBitSet::flag`] is
//!    recommended.
//!
//! # Memory layout
//!
//! Bitsets of small enough size are stored inline on the stack using a single `usize`, with the two
//! least-significant bits being used as metadata, like so:
//!
//! ```text
//!  field size = `usize::BITS` - 2
//!                     |
//!                     v
//! +----------------------+------+
//! |                 data | 0b01 | <--- metadata header
//! +----------------------+------+
//! ```
//!
//! Bitsets created using [`SmolBitSet::flag`] are also stored inline, but with different metadata
//! to signify a "sparse" representation:
//!
//! ```text
//!       represents `1 << flag`
//!                     |
//!                     v
//! +----------------------+------+
//! |                 flag | 0b11 | <--- metadata header
//! +----------------------+------+
//! ```
//!
//! Otherwise, bitsets which are too large to fit inline are allocated on the heap as a dynamic
//! array of `usize` elements, like so:
//!
//! ```text
//! +------+
//! |  ptr | <--- Guaranteed to be at least 32-bit aligned (i.e. metadata bits are 0b00)
//! +------+
//!     |
//!     v
//!   size        elements
//! +------+------+------+------+
//! |    3 |   b1 |   b2 |   b3 |
//! +------+------+------+------+
//! ```
//!
//! As such, a [`SmolBitSet`] has the same size as a `usize`, and additionally has a niche
//! optimization so that [`Option<SmolBitSet>`] is also the same size:
//!
//! | Target Pointer Size | [`size_of::<SmolBitSet>`] | Inline Capacity | Max Heap Capacity |
//! |--------------------:|--------------------------:|----------------:|------------------:|
//! | 32 bits             | 4 bytes                   | 30 bits         | 2^36 bits         |
//! | 64 bits             | 8 bytes                   | 62 bits         | 2^68 bits         |
//!
//! # Limitations
//!
//! * [`SmolBitSet`] does not implement [`Copy`].
//! * Implementing [`core::ops::Not`] is also not possible (or rather complex). Related alternative
//!   methods are provided via [`SmolBitSet::and_not`] and [`SmolBitSet::and_not_assign`].
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
    Inline = 0b01,
    Sparse = 0b11,
    Alloc = 0b00,
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

        res.reserve(b + 1);

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

    /// Clears the bitset, setting all bits to 0.
    ///
    /// Note that this has no effect on the allocated capacity of the bitset.
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// let mut sbs = SmolBitSet::new_inline(0b1010_0101);
    /// sbs.clear();
    /// assert!(sbs.is_empty());
    /// ```
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// let mut sbs = SmolBitSet::new_inline(0b1010_0101) << 64usize;
    /// sbs.clear();
    /// assert_eq!(sbs.capacity(), 128);
    /// assert!(sbs.is_empty());
    /// ```
    pub fn clear(&mut self) {
        if self.is_inline() {
            *self = Self::empty();
        } else {
            let data = unsafe { self.as_slice_mut_unchecked() };
            data.fill(0);
        }
    }

    #[inline]
    fn representation(&self) -> Representation {
        match self.ptr.addr().get() & 0b11 {
            0b00 => Representation::Alloc,
            0b01 => Representation::Inline,
            0b11 => Representation::Sparse,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn is_inline(&self) -> bool {
        matches!(
            self.representation(),
            Representation::Inline | Representation::Sparse
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
        matches!(self.representation(), Representation::Sparse)
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

    /// Returns the total number of bits the bitset can hold without reallocating.
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// let sbs = SmolBitSet::new_inline(0b1000_1001_1010_1101);
    /// assert_eq!(sbs.capacity(), usize::BITS as usize - 2);
    /// ```
    #[must_use]
    pub fn capacity(&self) -> usize {
        match self.representation() {
            Representation::Inline => MAX_INLINE_BITS,
            Representation::Alloc => {
                let size = unsafe { self.alloc_size_unchecked() };
                size * BITS
            }
            Representation::Sparse => {
                // Should this be `MAX_INLINE_BITS` instead?
                let flag = unsafe { self.get_sparse_data_unchecked() };
                flag + 1
            }
        }
    }

    /// Returns the number of bits in the bitset, not counting leading zeros.
    ///
    /// # Examples
    ///
    /// ```
    /// # use smolbitset::SmolBitSet;
    /// let sbs = SmolBitSet::new_inline(0b1000_1001_1010_1101);
    /// assert_eq!(sbs.len(), 16);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        match self.representation() {
            Representation::Inline => {
                let data = unsafe { self.get_inline_data_unchecked() };
                highest_set_bit!(usize, data).map_or(0, |b| b + 1)
            }
            Representation::Alloc => {
                let data = unsafe { self.as_slice_unchecked() };
                for (i, &data) in data.iter().enumerate().rev() {
                    if let Some(b) = highest_set_bit!(usize, data) {
                        return i * BITS + b + 1;
                    }
                }

                0
            }
            Representation::Sparse => {
                let flag = unsafe { self.get_sparse_data_unchecked() };
                flag + 1
            }
        }
    }

    /// Returns `true` if the bitset contains no set bits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the underlying data of the [`SmolBitSet`] split into individual [`usize`] blocks.
    #[must_use]
    pub fn data(&self) -> Cow<'_, [usize]> {
        if self.is_inline() {
            let data = unsafe { self.get_inline_data_unchecked() };
            vec![data].into()
        } else {
            let data = unsafe { self.as_slice_unchecked() };
            let len = self.len();
            let slice_len = if len == 0 { 1 } else { len.div_ceil(BITS) };
            data[..slice_len].into()
        }
    }

    #[inline]
    const unsafe fn alloc_size_unchecked(&self) -> usize {
        unsafe { *self.ptr.as_ptr() }
    }

    #[inline]
    const unsafe fn data_ptr_unchecked(&self) -> *mut usize {
        unsafe { self.ptr.as_ptr().add(1) }
    }

    #[inline]
    const unsafe fn as_slice_unchecked(&self) -> &[usize] {
        unsafe { slice::from_raw_parts(self.data_ptr_unchecked(), self.alloc_size_unchecked()) }
    }

    #[inline]
    const unsafe fn as_slice_mut_unchecked(&mut self) -> &mut [usize] {
        unsafe { slice::from_raw_parts_mut(self.data_ptr_unchecked(), self.alloc_size_unchecked()) }
    }

    fn normalize(&self) -> Self {
        if self.is_sparse() {
            let flag = unsafe { self.get_sparse_data_unchecked() };
            Self::new_inline(1) << flag
        } else {
            self.clone()
        }
    }

    /// Reserves capacity for at least `additional` more bits to be inserted in the given
    /// [`SmolBitSet`]. If capacity is already sufficient, this function does nothing.
    pub fn reserve(&mut self, additional: usize) {
        let new_capacity = self.len() + additional;
        if new_capacity > self.capacity() {
            let new_size = new_capacity.div_ceil(BITS);
            let new_layout = create_layout(new_size);

            let ptr = if self.is_inline() {
                // Create a new allocation and then copy data over
                #[expect(clippy::cast_ptr_alignment)]
                let ptr = unsafe { alloc::alloc(new_layout).cast::<usize>() };
                if ptr.is_null() {
                    handle_alloc_error(new_layout);
                }

                unsafe {
                    // Store the size in the first element
                    *ptr = new_size;
                    // Store inline data in the next element
                    *ptr.add(1) = self.get_inline_data_unchecked();
                    // Fill the rest with zeros
                    slice::from_raw_parts_mut(ptr.add(2), new_size - 1).fill(0);
                }
                ptr
            } else {
                let current_size = unsafe { self.alloc_size_unchecked() };
                debug_assert!(new_size >= current_size);

                // Reallocate the current data into a larger buffer
                let current_layout = create_layout(current_size);
                #[expect(clippy::cast_ptr_alignment)]
                let ptr = unsafe {
                    alloc::realloc(self.ptr.as_ptr().cast(), current_layout, new_layout.size())
                        .cast::<usize>()
                };
                if ptr.is_null() {
                    handle_alloc_error(new_layout);
                }

                unsafe {
                    // Update the new size in the first element
                    *ptr = new_size;
                    // Initialize newly allocated memory to zero
                    slice::from_raw_parts_mut(ptr.add(1 + current_size), new_size - current_size)
                        .fill(0);
                }
                ptr
            };

            self.ptr = unsafe { NonNull::new_unchecked(ptr) };
        }
    }
}

impl Drop for SmolBitSet {
    #[inline]
    fn drop(&mut self) {
        if self.is_inline() {
            return;
        }

        let alloc_size = unsafe { self.alloc_size_unchecked() };
        let layout = create_layout(alloc_size);

        unsafe {
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
        // `SmolBitSet::representation` will return `Representation::Alloc`. As it stands,
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

        t.reserve(0);
        assert!(t.is_inline());

        t.reserve(32);
        assert!(t.is_inline());

        let max_inline = MAX_INLINE_BITS;
        t.reserve(max_inline);
        assert!(t.is_inline());

        t.reserve(max_inline + 1);
        assert!(!t.is_inline());
        assert_eq!(t.capacity(), 64);

        t.reserve(65);
        assert!(!t.is_inline());
        assert_eq!(t.capacity(), 64 * 2);

        t.reserve(64 * 40);
        assert!(!t.is_inline());
        assert_eq!(t.capacity(), 64 * 40);
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
        assert_eq!(a.capacity(), 64);

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
    fn reserve() {
        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.reserve(30);
        assert!(sbs.is_inline());
        assert_eq!(sbs.capacity(), MAX_INLINE_BITS);

        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.reserve(55);
        assert!(sbs.is_inline());
        assert_eq!(sbs.capacity(), MAX_INLINE_BITS);

        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.reserve(64);
        assert!(!sbs.is_inline());
        assert_eq!(sbs.capacity(), 64);

        let mut sbs = SmolBitSet::empty();
        assert!(sbs.is_inline());

        sbs.reserve(65);
        assert!(!sbs.is_inline());
        assert_eq!(sbs.capacity(), 64 * 2);
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
