use crate::decoder::{Error, Result};

use super::vp8::TreeNode;

#[must_use]
#[repr(transparent)]
pub(crate) struct BitResult<T> {
    value_if_not_past_eof: T,
}

#[must_use]
pub(crate) struct BitResultAccumulator;

impl<T> BitResult<T> {
    const fn ok(value: T) -> Self {
        Self {
            value_if_not_past_eof: value,
        }
    }

    /// Instead of checking this result now, accumulate the burden of checking
    /// into an accumulator. This accumulator must be checked in the end.
    #[inline(always)]
    pub(crate) fn or_accumulate(self, acc: &mut BitResultAccumulator) -> T {
        let _ = acc;
        self.value_if_not_past_eof
    }
}

impl<T: Default> BitResult<T> {
    fn err() -> Self {
        Self {
            value_if_not_past_eof: T::default(),
        }
    }
}

#[cfg_attr(test, derive(Debug))]
pub(crate) struct ArithmeticDecoder {
    chunks: Box<[[u8; 4]]>,
    state: State,
    final_bytes: [u8; 3],
    final_bytes_remaining: i8,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy)]
struct State {
    chunk_index: usize,
    value: u64,
    range: u32,
    bit_count: i32,
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
            final_bytes: [0; 3],
            final_bytes_remaining: Self::FINAL_BYTES_REMAINING_EOF,
        }
    }

    pub(crate) fn init(&mut self, mut buf: Vec<[u8; 4]>, len: usize) -> Result<()> {
        let mut final_bytes = [0; 3];
        let final_bytes_remaining = if len == 4 * buf.len() {
            0
        } else {
            // Pop the last chunk (which is partial), then get length.
            let Some(last_chunk) = buf.pop() else {
                return Err(Error::NotEnoughInitData);
            };
            let len_rounded_down = 4 * buf.len();
            let num_bytes_popped = len - len_rounded_down;
            debug_assert!(num_bytes_popped <= 3);
            final_bytes[..num_bytes_popped].copy_from_slice(&last_chunk[..num_bytes_popped]);
            for i in num_bytes_popped..4 {
                debug_assert_eq!(last_chunk[i], 0, "unexpected {last_chunk:?}");
            }
            num_bytes_popped as i8
        };

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
            final_bytes,
            final_bytes_remaining,
        };
        Ok(())
    }

    /// Start a span of reading operations from the buffer, without stopping
    /// when the buffer runs out. For all valid webp images, the buffer will not
    /// run out prematurely. Conversely if the buffer ends early, the webp image
    /// cannot be correctly decoded and any intermediate results need to be
    /// discarded anyway.
    ///
    /// Each call to `start_accumulated_result` must be followed by a call to
    /// `check` on the *same* `ArithmeticDecoder`.
    #[inline(always)]
    pub(crate) fn start_accumulated_result(&mut self) -> BitResultAccumulator {
        BitResultAccumulator
    }

    /// Check that the read operations done so far were all valid.
    #[inline(always)]
    pub(crate) fn check<T>(
        &self,
        acc: BitResultAccumulator,
        value_if_not_past_eof: T,
    ) -> Result<T> {
        // The accumulator does not store any state because doing so is
        // too computationally expensive. Passing it around is a bit of
        // formality (that is optimized out) to ensure we call `check` .
        // Instead we check whether we have read past the end of the file.
        let BitResultAccumulator = acc;

        if self.is_past_eof() {
            Err(Error::BitStreamError)
        } else {
            Ok(value_if_not_past_eof)
        }
    }

    fn keep_accumulating<T>(
        &self,
        acc: BitResultAccumulator,
        value_if_not_past_eof: T,
    ) -> BitResult<T> {
        // The BitResult will be checked later by a different accumulator.
        // Because it does not carry state, that is fine.
        let BitResultAccumulator = acc;

        BitResult::ok(value_if_not_past_eof)
    }

    pub(crate) fn read_bool(&mut self, probability: u8) -> BitResult<bool> {
        self.cold_read_bool(probability)
    }

    pub(crate) fn read_flag(&mut self) -> BitResult<bool> {
        self.cold_read_flag()
    }

    pub(crate) fn read_literal(&mut self, n: u8) -> BitResult<u8> {
        self.cold_read_literal(n)
    }

    pub(crate) fn read_optional_signed_value(&mut self, n: u8) -> BitResult<i32> {
        self.cold_read_optional_signed_value(n)
    }

    pub(crate) fn read_with_tree<const N: usize>(&mut self, tree: &[TreeNode; N]) -> BitResult<i8> {
        let first_node = tree[0];
        self.read_with_tree_with_first_node(tree, first_node)
    }

    pub(crate) fn read_with_tree_with_first_node(
        &mut self,
        tree: &[TreeNode],
        first_node: TreeNode,
    ) -> BitResult<i8> {
        self.cold_read_with_tree(tree, usize::from(first_node.index))
    }

    const FINAL_BYTES_REMAINING_EOF: i8 = -0xE;

    fn load_from_final_bytes(&mut self) {
        match self.final_bytes_remaining {
            1.. => {
                self.final_bytes_remaining -= 1;
                let byte = self.final_bytes[0];
                self.final_bytes.rotate_left(1);
                self.state.value <<= 8;
                self.state.value |= u64::from(byte);
                self.state.bit_count += 8;
            }
            0 => {
                // libwebp seems to (sometimes?) allow bitstreams that read one byte past the end.
                // This replicates that logic.
                self.final_bytes_remaining -= 1;
                self.state.value <<= 8;
                self.state.bit_count += 8;
            }
            _ => {
                self.final_bytes_remaining = Self::FINAL_BYTES_REMAINING_EOF;
            }
        }
    }

    fn is_past_eof(&self) -> bool {
        self.final_bytes_remaining == Self::FINAL_BYTES_REMAINING_EOF
    }

    fn cold_read_bit(&mut self, probability: u8) -> BitResult<bool> {
        if self.state.bit_count < 0 {
            if let Some(chunk) = self.chunks.get(self.state.chunk_index).copied() {
                let v = u32::from_be_bytes(chunk);
                self.state.chunk_index += 1;
                self.state.value <<= 32;
                self.state.value |= u64::from(v);
                self.state.bit_count += 32;
            } else {
                self.load_from_final_bytes();
                if self.is_past_eof() {
                    return BitResult::err();
                }
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

        BitResult::ok(retval)
    }

    fn cold_read_bool(&mut self, probability: u8) -> BitResult<bool> {
        self.cold_read_bit(probability)
    }

    fn cold_read_flag(&mut self) -> BitResult<bool> {
        self.cold_read_bit(128)
    }

    fn cold_read_literal(&mut self, n: u8) -> BitResult<u8> {
        let mut v = 0u8;
        let mut res = self.start_accumulated_result();

        for _ in 0..n {
            let b = self.cold_read_flag().or_accumulate(&mut res);
            v = (v << 1) + u8::from(b);
        }

        self.keep_accumulating(res, v)
    }

    fn cold_read_optional_signed_value(&mut self, n: u8) -> BitResult<i32> {
        let mut res = self.start_accumulated_result();
        let flag = self.cold_read_flag().or_accumulate(&mut res);
        if !flag {
            // We should not read further bits if the flag is not set.
            return self.keep_accumulating(res, 0);
        }
        let magnitude = self.cold_read_literal(n).or_accumulate(&mut res);
        let sign = self.cold_read_flag().or_accumulate(&mut res);

        let value = if sign {
            -i32::from(magnitude)
        } else {
            i32::from(magnitude)
        };
        self.keep_accumulating(res, value)
    }

    fn cold_read_with_tree(&mut self, tree: &[TreeNode], start: usize) -> BitResult<i8> {
        let mut index = start;
        let mut res = self.start_accumulated_result();

        loop {
            let node = tree[index];
            let prob = node.prob;
            let b = self.cold_read_bit(prob).or_accumulate(&mut res);
            let t = if b { node.right } else { node.left };
            let new_index = usize::from(t);
            if new_index < tree.len() {
                index = new_index;
            } else {
                let value = TreeNode::value_from_branch(t);
                return self.keep_accumulating(res, value);
            }
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
        let mut buf = vec![[0u8; 4]; 1];
        buf.as_mut_slice().as_flattened_mut()[..size].copy_from_slice(&data[..]);
        decoder.init(buf, size).unwrap();
        let mut res = decoder.start_accumulated_result();
        assert_eq!(false, decoder.read_flag().or_accumulate(&mut res));
        assert_eq!(true, decoder.read_bool(10).or_accumulate(&mut res));
        assert_eq!(false, decoder.read_bool(250).or_accumulate(&mut res));
        assert_eq!(1, decoder.read_literal(1).or_accumulate(&mut res));
        assert_eq!(5, decoder.read_literal(3).or_accumulate(&mut res));
        assert_eq!(64, decoder.read_literal(8).or_accumulate(&mut res));
        assert_eq!(185, decoder.read_literal(8).or_accumulate(&mut res));
        decoder.check(res, ()).unwrap();
    }

    #[test]
    fn test_arithmetic_decoder_hello_long() {
        let mut decoder = ArithmeticDecoder::new();
        let data = b"hello world";
        let size = data.len();
        let mut buf = vec![[0u8; 4]; (size + 3) / 4];
        buf.as_mut_slice().as_flattened_mut()[..size].copy_from_slice(&data[..]);
        decoder.init(buf, size).unwrap();
        let mut res = decoder.start_accumulated_result();
        assert_eq!(false, decoder.read_flag().or_accumulate(&mut res));
        assert_eq!(true, decoder.read_bool(10).or_accumulate(&mut res));
        assert_eq!(false, decoder.read_bool(250).or_accumulate(&mut res));
        assert_eq!(1, decoder.read_literal(1).or_accumulate(&mut res));
        assert_eq!(5, decoder.read_literal(3).or_accumulate(&mut res));
        assert_eq!(64, decoder.read_literal(8).or_accumulate(&mut res));
        assert_eq!(185, decoder.read_literal(8).or_accumulate(&mut res));
        assert_eq!(31, decoder.read_literal(8).or_accumulate(&mut res));
        decoder.check(res, ()).unwrap();
    }

    #[test]
    fn test_arithmetic_decoder_uninit() {
        let mut decoder = ArithmeticDecoder::new();
        let mut res = decoder.start_accumulated_result();
        let _ = decoder.read_flag().or_accumulate(&mut res);
        let result = decoder.check(res, ());
        assert!(result.is_err());
    }
}
