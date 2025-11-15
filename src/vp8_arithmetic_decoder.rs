use crate::decoder::DecodingError;

use super::vp8::TreeNode;

#[cfg_attr(test, derive(Debug))]
pub(crate) struct ArithmeticDecoder {
    chunks: Box<[[u8; 4]]>,
    state: State,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy)]
struct State {
    chunk_index: usize,
    value: u64,
    range: u32,
    bit_count: i32,
}

#[cfg_attr(test, derive(Debug))]
struct FastDecoder<'a> {
    chunks: &'a [[u8; 4]],
    uncommitted_state: State,
    save_state: &'a mut State,
}

impl ArithmeticDecoder {
    pub(crate) fn new() -> ArithmeticDecoder {
        let state = State {
            chunk_index: 0,
            value: 0,
            range: 255,
            bit_count: -8,
        };
        ArithmeticDecoder {
            chunks: Box::new([]),
            state,
        }
    }

    pub(crate) fn init(&mut self, buf: Vec<[u8; 4]>) -> Result<(), DecodingError> {
        let chunks = buf.into_boxed_slice();
        let state = State {
            chunk_index: 0,
            value: 0,
            range: 255,
            bit_count: -8,
        };
        *self = Self {
            chunks,
            state,
        };
        Ok(())
    }

    /// Check that the read operations done so far were all valid.
    #[inline(always)]
    pub(crate) fn check<T>(
        &self,
        value: T,
    ) -> Result<T, DecodingError> {
        if self.state.chunk_index > self.chunks.len() {
            Err(DecodingError::BitStreamError)
        } else {
            Ok(value)
        }
    }

    // Do not inline this because inlining seems to worsen performance.
    #[inline(never)]
    pub(crate) fn read_bool(&mut self, probability: u8) -> bool {
        if let Some(b) = self.fast().read_bool(probability) {
            return b;
        }

        self.cold_read_bool(probability)
    }

    // Do not inline this because inlining seems to worsen performance.
    #[inline(never)]
    pub(crate) fn read_flag(&mut self) -> bool {
        if let Some(b) = self.fast().read_flag() {
            return b;
        }

        self.cold_read_flag()
    }

    // Do not inline this because inlining seems to worsen performance.
    #[inline(never)]
    pub(crate) fn read_sign(&mut self) -> bool {
        if let Some(b) = self.fast().read_sign() {
            return b;
        }

        self.cold_read_flag()
    }

    // Do not inline this because inlining seems to worsen performance.
    #[inline(never)]
    pub(crate) fn read_literal(&mut self, n: u8) -> u8 {
        if let Some(v) = self.fast().read_literal(n) {
            return v;
        }

        self.cold_read_literal(n)
    }

    // Do not inline this because inlining seems to worsen performance.
    #[inline(never)]
    pub(crate) fn read_optional_signed_value(&mut self, n: u8) -> i32 {
        if let Some(v) = self.fast().read_optional_signed_value(n) {
            return v;
        }

        self.cold_read_optional_signed_value(n)
    }

    // This is generic and inlined just to skip the first bounds check.
    #[inline]
    pub(crate) fn read_with_tree<const N: usize>(&mut self, tree: &[TreeNode; N]) -> i8 {
        let first_node = tree[0];
        self.read_with_tree_with_first_node(tree, first_node)
    }

    // Do not inline this because inlining significantly worsens performance.
    #[inline(never)]
    pub(crate) fn read_with_tree_with_first_node(
        &mut self,
        tree: &[TreeNode],
        first_node: TreeNode,
    ) -> i8 {
        if let Some(v) = self.fast().read_with_tree(tree, first_node) {
            return v;
        }

        self.cold_read_with_tree(tree, usize::from(first_node.index))
    }

    // As a similar (but different) speedup to BitResult, the FastDecoder reads
    // bits under an assumption and validates it at the end.
    //
    // The idea here is that for normal-sized webp images, the vast majority
    // of bits are somewhere other than in the last four bytes. Therefore we
    // can pretend the buffer has infinite size. After we are done reading,
    // we check if we actually read past the end of `self.chunks`.
    // If so, we backtrack (or rather we discard `uncommitted_state`)
    // and try again with the slow approach. This might result in doing double
    // work for those last few bytes -- in fact we even keep retrying the fast
    // method to save an if-statement --, but more than make up for that by
    // speeding up reading from the other thousands or millions of bytes.
    fn fast(&mut self) -> FastDecoder<'_> {
        FastDecoder {
            chunks: &self.chunks,
            uncommitted_state: self.state,
            save_state: &mut self.state,
        }
    }

    fn cold_read_bit(&mut self, probability: u8) -> bool {
        if self.state.bit_count < 0 {
            if let Some(chunk) = self.chunks.get(self.state.chunk_index).copied() {
                let v = u32::from_be_bytes(chunk);
                self.state.chunk_index += 1;
                self.state.value <<= 32;
                self.state.value |= u64::from(v);
                self.state.bit_count += 32;
            } else {
                self.state.chunk_index += 1;
                self.state.bit_count = 0;
            }
        }
        debug_assert!(self.state.bit_count >= 0);

        let probability = u32::from(probability);
        let split = 1 + (((self.state.range - 1) * probability) >> 8);
        let bigsplit = u64::from(split) << self.state.bit_count;

        let retval = if let Some(new_value) = self.state.value.checked_sub(bigsplit) {
            self.state.range -= split;
            self.state.value = new_value;
            true
        } else {
            self.state.range = split;
            false
        };
        debug_assert!(self.state.range > 0);

        // Compute shift required to satisfy `self.state.range >= 128`.
        // Apply that shift to `self.state.range` and `self.state.bitcount`.
        //
        // Subtract 24 because we only care about leading zeros in the
        // lowest byte of `self.state.range` which is a `u32`.
        let shift = self.state.range.leading_zeros().saturating_sub(24);
        self.state.range <<= shift;
        self.state.bit_count -= shift as i32;
        debug_assert!(self.state.range >= 128);

        retval
    }

    #[cold]
    #[inline(never)]
    fn cold_read_bool(&mut self, probability: u8) -> bool {
        self.cold_read_bit(probability)
    }

    #[cold]
    #[inline(never)]
    fn cold_read_flag(&mut self) -> bool {
        self.cold_read_bit(128)
    }

    #[cold]
    #[inline(never)]
    fn cold_read_literal(&mut self, n: u8) -> u8 {
        let mut v = 0u8;

        for _ in 0..n {
            let b = self.cold_read_flag();
            v = (v << 1) + u8::from(b);
        }

        v
    }

    #[cold]
    #[inline(never)]
    fn cold_read_optional_signed_value(&mut self, n: u8) -> i32 {
        let flag = self.cold_read_flag();
        if !flag {
            return 0;
        }
        let magnitude = self.cold_read_literal(n);
        let sign = self.cold_read_flag();

        if sign {
            -i32::from(magnitude)
        } else {
            i32::from(magnitude)
        }
    }

    #[cold]
    #[inline(never)]
    fn cold_read_with_tree(&mut self, tree: &[TreeNode], start: usize) -> i8 {
        let mut index = start;

        loop {
            let node = tree[index];
            let prob = node.prob;
            let b = self.cold_read_bit(prob);
            let t = if b { node.right } else { node.left };
            let new_index = usize::from(t);
            if new_index < tree.len() {
                index = new_index;
            } else {
                let value = TreeNode::value_from_branch(t);
                return value;
            }
        }
    }
}

impl FastDecoder<'_> {
    fn commit_if_valid<T>(self, value_if_not_past_eof: T) -> Option<T> {
        // If `chunk_index > self.chunks.len()`, it means we used zeroes
        // instead of an actual chunk and `value_if_not_past_eof` is nonsense.
        if self.uncommitted_state.chunk_index <= self.chunks.len() {
            *self.save_state = self.uncommitted_state;
            Some(value_if_not_past_eof)
        } else {
            None
        }
    }

    fn read_bool(mut self, probability: u8) -> Option<bool> {
        let bit = self.fast_read_bit(probability);
        self.commit_if_valid(bit)
    }

    fn read_flag(mut self) -> Option<bool> {
        let value = self.fast_read_flag();
        self.commit_if_valid(value)
    }

    fn read_sign(mut self) -> Option<bool> {
        let value = self.fast_read_sign();
        self.commit_if_valid(value)
    }

    fn read_literal(mut self, n: u8) -> Option<u8> {
        let value = self.fast_read_literal(n);
        self.commit_if_valid(value)
    }

    fn read_optional_signed_value(mut self, n: u8) -> Option<i32> {
        let flag = self.fast_read_flag();
        if !flag {
            // We should not read further bits if the flag is not set.
            return self.commit_if_valid(0);
        }
        let magnitude = self.fast_read_literal(n);
        let sign = self.fast_read_flag();
        let value = if sign {
            -i32::from(magnitude)
        } else {
            i32::from(magnitude)
        };
        self.commit_if_valid(value)
    }

    fn read_with_tree(mut self, tree: &[TreeNode], first_node: TreeNode) -> Option<i8> {
        let value = self.fast_read_with_tree(tree, first_node);
        self.commit_if_valid(value)
    }

    fn fast_read_bit(&mut self, probability: u8) -> bool {
        let State {
            mut chunk_index,
            mut value,
            mut range,
            mut bit_count,
        } = self.uncommitted_state;

        if bit_count < 0 {
            let chunk = self.chunks.get(chunk_index).copied();
            // We ignore invalid data inside the `fast_` functions,
            // but we increase `chunk_index` below, so we can check
            // whether we read invalid data in `commit_if_valid`.
            let chunk = chunk.unwrap_or_default();

            let v = u32::from_be_bytes(chunk);
            chunk_index += 1;
            value <<= 32;
            value |= u64::from(v);
            bit_count += 32;
        }
        debug_assert!(bit_count >= 0);

        debug_assert!((128..=255).contains(&range));
        let probability = u32::from(probability);
        let split = 1 + (((range - 1) * probability) >> 8);
        let bigsplit = u64::from(split) << bit_count;

        let retval = if let Some(new_value) = value.checked_sub(bigsplit) {
            range -= split;
            value = new_value;
            true
        } else {
            range = split;
            false
        };

        // Compute shift required to satisfy `range >= 128`.
        // Apply that shift to `range` and `self.bitcount`.
        //
        // Subtract 24 because we only care about leading zeros in the
        // lowest byte of `range` which is a `u32`.
        debug_assert!((1..=254).contains(&range));
        let shift = range.leading_zeros().saturating_sub(24);
        range <<= shift;
        bit_count -= shift as i32;

        debug_assert!((128..=254).contains(&range));
        self.uncommitted_state = State {
            chunk_index,
            value,
            range,
            bit_count,
        };
        retval
    }

    fn fast_read_flag(&mut self) -> bool {
        let State {
            mut chunk_index,
            mut value,
            mut range,
            mut bit_count,
        } = self.uncommitted_state;

        if bit_count < 0 {
            let chunk = self.chunks.get(chunk_index).copied();
            // We ignore invalid data inside the `fast_` functions,
            // but we increase `chunk_index` below, so we can check
            // whether we read invalid data in `commit_if_valid`.
            let chunk = chunk.unwrap_or_default();

            let v = u32::from_be_bytes(chunk);
            chunk_index += 1;
            value <<= 32;
            value |= u64::from(v);
            bit_count += 32;
        }
        debug_assert!(bit_count >= 0);

        debug_assert!((128..=255).contains(&range));
        let half_range = range / 2;
        let split = range - half_range;
        let bigsplit = u64::from(split) << bit_count;

        let retval = if let Some(new_value) = value.checked_sub(bigsplit) {
            range = half_range;
            value = new_value;
            true
        } else {
            range = split;
            false
        };

        // Compute shift required to satisfy `range >= 128`.
        // A `range` of 64..127 requires a shift of 1. No shift if `range` is 128.
        // Apply that shift to `range` and `self.bitcount`.
        debug_assert!((64..=128).contains(&range));
        let shift = if range == 0x80 { 0 } else { 1 };
        range <<= shift;
        bit_count -= shift;

        debug_assert!((128..=254).contains(&range));
        self.uncommitted_state = State {
            chunk_index,
            value,
            range,
            bit_count,
        };
        retval
    }

    fn fast_read_sign(&mut self) -> bool {
        let State {
            mut chunk_index,
            mut value,
            mut range,
            mut bit_count,
        } = self.uncommitted_state;

        if bit_count < 0 {
            let chunk = self.chunks.get(chunk_index).copied();
            // We ignore invalid data inside the `fast_` functions,
            // but we increase `chunk_index` below, so we can check
            // whether we read invalid data in `commit_if_valid`.
            let chunk = chunk.unwrap_or_default();

            let v = u32::from_be_bytes(chunk);
            chunk_index += 1;
            value <<= 32;
            value |= u64::from(v);
            bit_count += 32;
        }

        // Range is only 255 at the start of decoding. After reading any symbol, it is guaranteed
        // to be at most 254. Sign bits are never the first symbol in a bit stream.
        debug_assert!((128..=254).contains(&range));
        let half_range = range / 2;
        let split = range - half_range;
        let bigsplit = u64::from(split) << bit_count;

        let retval = if let Some(new_value) = value.checked_sub(bigsplit) {
            range = half_range;
            value = new_value;
            true
        } else {
            range = split;
            false
        };

        // Compute shift required to satisfy `range >= 128`.
        // Since `range` lies in 64..127 it always requires a shift of 1.
        // Apply that shift to `range` and `self.bitcount`.
        debug_assert!((64..=127).contains(&range));
        range <<= 1;
        bit_count -= 1;

        debug_assert!((128..=254).contains(&range));
        self.uncommitted_state = State {
            chunk_index,
            value,
            range,
            bit_count,
        };
        retval
    }

    fn fast_read_literal(&mut self, n: u8) -> u8 {
        let mut v = 0u8;
        for _ in 0..n {
            let b = self.fast_read_flag();
            v = (v << 1) + u8::from(b);
        }
        v
    }

    fn fast_read_with_tree(&mut self, tree: &[TreeNode], mut node: TreeNode) -> i8 {
        loop {
            let prob = node.prob;
            let b = self.fast_read_bit(prob);
            let i = if b { node.right } else { node.left };
            let Some(next_node) = tree.get(usize::from(i)) else {
                return TreeNode::value_from_branch(i);
            };
            node = *next_node;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arithmetic_decoder_hello_short() {
        let mut decoder = ArithmeticDecoder::new();
        let data = b"hel";
        let size = data.len();
        let mut buf = vec![[0u8; 4]; size.div_ceil(4)];
        buf.as_mut_slice().as_flattened_mut()[..size].copy_from_slice(&data[..]);
        decoder.init(buf).unwrap();
        assert_eq!(false, decoder.read_flag());
        assert_eq!(true, decoder.read_bool(10));
        assert_eq!(false, decoder.read_bool(250));
        assert_eq!(1, decoder.read_literal(1));
        assert_eq!(5, decoder.read_literal(3));
        assert_eq!(64, decoder.read_literal(8));
        assert_eq!(185, decoder.read_literal(8));
    }

    #[test]
    fn test_arithmetic_decoder_hello_long() {
        let mut decoder = ArithmeticDecoder::new();
        let data = b"hello world";
        let size = data.len();
        let mut buf = vec![[0u8; 4]; size.div_ceil(4)];
        buf.as_mut_slice().as_flattened_mut()[..size].copy_from_slice(&data[..]);
        decoder.init(buf).unwrap();
        assert_eq!(false, decoder.read_flag());
        assert_eq!(true, decoder.read_bool(10));
        assert_eq!(false, decoder.read_bool(250));
        assert_eq!(1, decoder.read_literal(1));
        assert_eq!(5, decoder.read_literal(3));
        assert_eq!(64, decoder.read_literal(8));
        assert_eq!(185, decoder.read_literal(8));
        assert_eq!(31, decoder.read_literal(8));
        decoder.check(()).unwrap();
    }

    #[test]
    fn test_arithmetic_decoder_uninit() {
        let mut decoder = ArithmeticDecoder::new();
        let _ = decoder.read_flag();
        let result = decoder.check(());
        assert!(result.is_err());
    }
}
