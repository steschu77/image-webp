use super::vp8::TreeNode;

pub(crate) struct ArithmeticDecoder<'a> {
    chunks: &'a [[u8; 4]],
    last_chunk: [u8; 4],
    chunk_index: usize,
    chunk_count: usize,
    state: State,
}

#[derive(Clone, Copy)]
struct State {
    value: u64,
    range: u32,
    bit_count: i32,
}

impl<'a> ArithmeticDecoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let (chunks, last) = data.as_chunks::<4>();
        let mut last_chunk = [0u8; 4];
        last_chunk[..last.len()].copy_from_slice(last);
        let chunk_count = data.len().div_ceil(4);
        
        let state = State {
            value: 0,
            range: 255,
            bit_count: -8,
        };

        Self {
            chunks,
            last_chunk,
            chunk_index: 0,
            chunk_count,
            state,
        }
    }

    pub(crate) fn is_overflow(&self) -> bool {
        self.chunk_index > self.chunk_count
    }

    fn refill_bits(&mut self) {
        let chunk = self.chunks.get(self.chunk_index).copied().unwrap_or(self.last_chunk);
        self.chunk_index += 1;
        self.state.value = (self.state.value << 32) | u64::from(u32::from_be_bytes(chunk));
        self.state.bit_count += 32;
    }

    //#[inline(never)]
    fn read_bit(&mut self, probability: u32) -> bool {
        if self.state.bit_count < 0 {
            self.refill_bits();
        }

        let split = 1 + (((self.state.range - 1) * probability) >> 8);
        let value_split = u64::from(split) << self.state.bit_count;
        let value = self.state.value.checked_sub(value_split);

        if let Some(value) = value {
            self.state.range -= split;
            self.state.value = value;
        } else {
            self.state.range = split;
        }

        let shift = self.state.range.leading_zeros().saturating_sub(24);
        self.state.range <<= shift;
        self.state.bit_count -= shift as i32;

        value.is_some()
    }

    //#[inline(never)]
    pub(crate) fn read_flag(&mut self) -> bool {
        if self.state.bit_count < 0 {
            self.refill_bits();
        }

        let split = 1 + ((self.state.range - 1) >> 1);
        let value_split = u64::from(split) << self.state.bit_count;
        let value = self.state.value.checked_sub(value_split);

        if let Some(value) = value {
            self.state.range -= split;
            self.state.value = value;
        } else {
            self.state.range = split;
        }

        let shift = self.state.range.leading_zeros().saturating_sub(24);
        self.state.range <<= shift;
        self.state.bit_count -= shift as i32;

        value.is_some()
    }

    //#[inline(never)]
    pub(crate) fn read_signed(&mut self, abs_value: i32) -> i32 {
        if self.state.bit_count < 0 {
            self.refill_bits();
        }

        let split_32 = (self.state.range + 1) >> 1;
        let split_64 = u64::from(split_32) << self.state.bit_count;
        let value = self.state.value.checked_sub(split_64);

        if let Some(value) = value {
            self.state.range -= split_32;
            self.state.value = value;
        } else {
            self.state.range = split_32;
        }

        self.state.range <<= 1;
        self.state.bit_count -= 1;

        if value.is_some() {
            -abs_value
        } else {
            abs_value
        }
    }

    pub(crate) fn read_bool(&mut self, probability: u8) -> bool {
        self.read_bit(probability as u32)
    }

    pub(crate) fn read_literal(&mut self, n: u8) -> u8 {
        (0..n).fold(0u8, |v, _| (v << 1) | u8::from(self.read_flag()))
    }

    pub(crate) fn read_optional_signed_value(&mut self, n: u8) -> i32 {
        let flag = self.read_flag();
        if !flag {
            return 0;
        }
        let magnitude = self.read_literal(n) as i32;
        self.read_signed(magnitude)
    }

    pub(crate) fn read_with_tree<const N: usize>(&mut self, tree: &[TreeNode; N]) -> i8 {
        self.read_with_tree_with_first_node(tree, tree[0])
    }

    //#[inline(never)]
    pub(crate) fn read_with_tree_with_first_node(
        &mut self,
        tree: &[TreeNode],
        first_node: TreeNode,
    ) -> i8 {
        let start = usize::from(first_node.index);
        let mut index = start;

        loop {
            let node = tree[index];
            let b = self.read_bit(node.prob as u32);
            let t = node.children[b as usize];
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
        let data = b"hel";
        let mut decoder = ArithmeticDecoder::new(data);
        assert_eq!(false, decoder.read_flag());
        assert_eq!(true, decoder.read_bool(10));
        assert_eq!(false, decoder.read_bool(250));
        assert_eq!(1, decoder.read_literal(1));
        assert_eq!(5, decoder.read_literal(3));
        assert_eq!(64, decoder.read_literal(8));
        assert_eq!(185, decoder.read_literal(8));
        assert!(!decoder.is_overflow());
    }

    #[test]
    fn test_arithmetic_decoder_hello_long() {
        let data = b"hello world";
        let mut decoder = ArithmeticDecoder::new(data);
        assert_eq!(false, decoder.read_flag());
        assert_eq!(true, decoder.read_bool(10));
        assert_eq!(false, decoder.read_bool(250));
        assert_eq!(1, decoder.read_literal(1));
        assert_eq!(5, decoder.read_literal(3));
        assert_eq!(64, decoder.read_literal(8));
        assert_eq!(185, decoder.read_literal(8));
        assert_eq!(31, decoder.read_literal(8));
        assert!(!decoder.is_overflow());
    }

    #[test]
    fn test_arithmetic_decoder_uninit() {
        let data = b"";
        let mut decoder = ArithmeticDecoder::new(data);
        let _ = decoder.read_flag();
        assert!(decoder.is_overflow());
    }
}
