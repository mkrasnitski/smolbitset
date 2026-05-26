use crate::SmolBitSet;

pub enum BstSlice<'a> {
    Inline(usize),
    Heap(&'a [usize]),
}

impl<'a> BstSlice<'a> {
    pub fn new(sbs: &'a SmolBitSet) -> Self {
        debug_assert!(!sbs.is_sparse());

        if sbs.is_inline() {
            let data = unsafe { sbs.get_inline_data_unchecked() };
            Self::Inline(data)
        } else {
            let slice = unsafe { sbs.as_slice_unchecked() };
            Self::Heap(slice)
        }
    }

    pub const fn slice(&self) -> &[usize] {
        match self {
            Self::Inline(data) => core::slice::from_ref(data),
            Self::Heap(slice) => slice,
        }
    }
}
