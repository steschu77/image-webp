use crate::decoder::{Error, Result};

use super::vp8::TreeNode;

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

    fn read_bit(&mut self, probability: u8) -> bool {
        if self.state.bit_count < 0 {
            if let Some(chunk) = self.chunks.get(self.state.chunk_index).copied() {
                let v = u32::from_be_bytes(chunk);
                self.state.chunk_index += 1;
                self.state.value <<= 32;
                self.state.value |= u64::from(v);
                self.state.bit_count += 32;
            } else {
                self.load_from_final_bytes();
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

    pub(crate) fn read_bool(&mut self, probability: u8) -> bool {
        self.read_bit(probability)
    }

    pub(crate) fn read_flag(&mut self) -> bool {
        self.read_bit(128)
    }

    pub(crate) fn read_literal(&mut self, n: u8) -> u8 {
        let mut v = 0u8;

        for _ in 0..n {
            let b = self.read_flag();
            v = (v << 1) + u8::from(b);
        }

        v
    }

    pub(crate) fn read_optional_signed_value(&mut self, n: u8) -> i32 {
        let flag = self.read_flag();
        if !flag {
            return 0;
        }
        let magnitude = self.read_literal(n);
        let sign = self.read_flag();

        if sign {
            -i32::from(magnitude)
        } else {
            i32::from(magnitude)
        }
    }

    pub(crate) fn read_with_tree<const N: usize>(&mut self, tree: &[TreeNode; N]) -> i8 {
        self.read_with_tree_with_first_node(tree, tree[0])
    }

    pub(crate) fn read_with_tree_with_first_node(
        &mut self,
        tree: &[TreeNode],
        first_node: TreeNode,
    ) -> i8 {
        let start = usize::from(first_node.index);
        let mut index = start;

        loop {
            let node = tree[index];
            let prob = node.prob;
            let b = self.read_bit(prob);
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
        let mut buf = vec![[0u8; 4]; (size + 3) / 4];
        buf.as_mut_slice().as_flattened_mut()[..size].copy_from_slice(&data[..]);
        decoder.init(buf, size).unwrap();
        assert_eq!(false, decoder.read_flag());
        assert_eq!(true, decoder.read_bool(10));
        assert_eq!(false, decoder.read_bool(250));
        assert_eq!(1, decoder.read_literal(1));
        assert_eq!(5, decoder.read_literal(3));
        assert_eq!(64, decoder.read_literal(8));
        assert_eq!(185, decoder.read_literal(8));
        assert_eq!(31, decoder.read_literal(8));
    }
}
