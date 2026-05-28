use crate::SmolBitSet;

impl typesize::TypeSize for SmolBitSet {
    fn extra_size(&self) -> usize {
        const ELEM_SIZE: usize = core::mem::size_of::<usize>();

        if self.is_inline() {
            0
        } else {
            let len = unsafe { self.alloc_size_unchecked() };
            ELEM_SIZE * (len + 1)
        }
    }
}
