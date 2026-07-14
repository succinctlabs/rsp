use bumpalo::Bump;
use bytes::{buf::UninitSlice, BufMut, TryGetError};

fn panic_advance(error_info: &TryGetError) -> ! {
    panic!(
        "advance out of bounds: the len is {} but advancing by {}",
        error_info.available, error_info.requested
    );
}

/// A helper type to implement bytes::BufMut for bumpalo::collections::Vec<'a, u8>.
pub(crate) struct BumpBytesMut<'a> {
    inner: bumpalo::collections::Vec<'a, u8>,
}

impl<'a> BumpBytesMut<'a> {
    #[inline(always)]
    pub(crate) fn with_capacity_in(capacity: usize, bump: &'a Bump) -> Self {
        Self { inner: bumpalo::collections::Vec::with_capacity_in(capacity, bump) }
    }

    #[inline(always)]
    pub(crate) fn into_inner(self) -> bumpalo::collections::Vec<'a, u8> {
        self.inner
    }

    #[inline(always)]
    pub(crate) fn len(&self) -> usize {
        self.inner.len()
    }
}

/// This implementation is taken from:
/// https://github.com/tokio-rs/bytes/blob/4b53a29eb26716592ef2f00f925ef58ccb182e61/src/buf/buf_mut.rs#L1599
unsafe impl BufMut for BumpBytesMut<'_> {
    #[inline(always)]
    fn remaining_mut(&self) -> usize {
        isize::MAX as usize - self.inner.len()
    }

    #[inline(always)]
    unsafe fn advance_mut(&mut self, cnt: usize) {
        let len = self.inner.len();
        let remaining = self.inner.capacity() - len;

        if remaining < cnt {
            panic_advance(&TryGetError { requested: cnt, available: remaining });
        }

        // SAFETY: Addition will not overflow since the sum is at most the capacity.
        unsafe {
            self.inner.set_len(len + cnt);
        }
    }

    #[inline(always)]
    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        if self.inner.capacity() == self.inner.len() {
            self.inner.reserve(64); // Grow the vec
        }

        let cap = self.inner.capacity();
        let len = self.inner.len();

        let ptr = self.inner.as_mut_ptr();
        // SAFETY: Since `ptr` is valid for `cap` bytes, `ptr.add(len)` must be
        // valid for `cap - len` bytes. The subtraction will not underflow since
        // `len <= cap`.
        unsafe { UninitSlice::from_raw_parts_mut(ptr.add(len), cap - len) }
    }

    // Specialize these methods so they can skip checking `remaining_mut`
    // and `advance_mut`.
    #[inline]
    fn put<T: bytes::Buf>(&mut self, mut src: T)
    where
        Self: Sized,
    {
        // In case the src isn't contiguous, reserve upfront.
        self.inner.reserve(src.remaining());

        while src.has_remaining() {
            let s = src.chunk();
            let l = s.len();
            self.inner.extend_from_slice(s);
            src.advance(l);
        }
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        self.inner.extend_from_slice(src);
    }

    #[inline]
    fn put_bytes(&mut self, val: u8, cnt: usize) {
        // If the addition overflows, then the `resize` will fail.
        let new_len = self.len().saturating_add(cnt);
        self.inner.resize(new_len, val);
    }
}
