use alloc::{vec, vec::Vec};

use super::{LzEncoder, MatchFind, Matches, extend_match, hash234::Hash234};

/// Binary Tree with 4-byte matching
pub(crate) struct Bt4 {
    hash: Hash234,
    tree: Vec<i32>,
    depth_limit: i32,
    cyclic_size: i32,
    cyclic_pos: i32,
    lz_pos: i32,
    // A nice-length tree match can continue at the next position when the
    // match distance is unchanged and its one newly exposed byte also matches.
    continuation_pos: i32,
    continuation_delta: i32,
    continuation_nice_len: i32,
}

const MAX_POS: i32 = 0x7FFFFFFF;

#[inline(always)]
fn sh_left(i: i32) -> i32 {
    ((i as u32) << 1) as i32
}

impl Bt4 {
    pub(crate) fn new(dict_size: u32, nice_len: u32, depth_limit: i32) -> Self {
        let cyclic_size = dict_size as i32 + 1;

        let tree = vec![0; cyclic_size as usize * 2];

        Self {
            hash: Hash234::new(dict_size),
            tree,
            depth_limit: if depth_limit > 0 {
                depth_limit
            } else {
                16 + nice_len as i32 / 2
            },
            cyclic_size,
            cyclic_pos: -1,
            lz_pos: cyclic_size,
            continuation_pos: -2,
            continuation_delta: 0,
            continuation_nice_len: 0,
        }
    }

    pub(crate) fn get_mem_usage(dict_size: u32) -> u32 {
        Hash234::get_mem_usage(dict_size) + dict_size / (1024 / 8) + 10
    }

    fn move_pos(&mut self, encoder: &mut super::LzEncoderData) -> i32 {
        let avail = encoder.move_pos(encoder.nice_len as _, 4);
        if avail != 0 {
            self.lz_pos += 1;
            if self.lz_pos == MAX_POS {
                self.normalize_positions();
            }
            self.cyclic_pos += 1;
            if self.cyclic_pos == self.cyclic_size {
                self.cyclic_pos = 0;
            }
        }
        avail
    }

    #[cold]
    #[inline(never)]
    fn normalize_positions(&mut self) {
        self.continuation_pos = -2;
        let normalization_offset = MAX_POS - self.cyclic_size;
        self.hash.normalize(normalization_offset);
        LzEncoder::normalize(&mut self.tree, normalization_offset);
        self.lz_pos -= normalization_offset;
    }

    fn skip_tree(
        &mut self,
        encoder: &mut super::LzEncoderData,
        nice_len_limit: i32,
        mut current_match: i32,
    ) {
        let delta = self.lz_pos - current_match;
        if self.continuation_pos + 1 == self.lz_pos
            && self.continuation_delta == delta
            && self.continuation_nice_len == nice_len_limit
            && delta < self.cyclic_size
            && encoder.get_avail() >= nice_len_limit
            && encoder.get_byte((nice_len_limit - 1) as _, delta)
                == encoder.get_byte((nice_len_limit - 1) as _, 0)
        {
            // The prior position proved nice_len_limit bytes. After shifting
            // one byte, only the final byte is new; matching it makes this
            // position a nice-length match too. Preserve the ordinary terminal
            // tree splice while avoiding the search that established it.
            let pair_selector = self.cyclic_size * ((delta > self.cyclic_pos) as i32);
            let pair = sh_left(self.cyclic_pos - delta + pair_selector);
            let ptr0 = sh_left(self.cyclic_pos) + 1;
            let ptr1 = sh_left(self.cyclic_pos);
            self.tree[ptr1 as usize] = self.tree[pair as usize];
            self.tree[ptr0 as usize] = self.tree[pair as usize + 1];
            self.continuation_pos = self.lz_pos;
            return;
        }
        self.continuation_pos = -2;

        let mut depth = self.depth_limit;

        let mut ptr0 = sh_left(self.cyclic_pos) + 1;
        let mut ptr1 = sh_left(self.cyclic_pos);
        let mut len0 = 0;
        let mut len1 = 0;

        loop {
            let delta = self.lz_pos - current_match;

            if depth == 0 || delta >= self.cyclic_size {
                self.tree[ptr0 as usize] = 0;
                self.tree[ptr1 as usize] = 0;
                return;
            }
            depth -= 1;

            let pair_selector = self.cyclic_size * ((delta > self.cyclic_pos) as i32);
            let pair = sh_left(self.cyclic_pos - delta + pair_selector);

            let mut len = len0.min(len1);

            // Tree maintenance needs only the nice-length prefix.
            len = extend_match(&encoder.buf, encoder.read_pos, len, delta, nice_len_limit);
            if len == nice_len_limit {
                self.tree[ptr1 as usize] = self.tree[pair as usize];
                self.tree[ptr0 as usize] = self.tree[pair as usize + 1];
                self.continuation_pos = self.lz_pos;
                self.continuation_delta = delta;
                self.continuation_nice_len = nice_len_limit;
                return;
            }

            if encoder.get_byte(len as _, delta) < encoder.get_byte(len as _, 0) {
                self.tree[ptr1 as usize] = current_match;
                ptr1 = pair + 1;
                current_match = self.tree[ptr1 as usize];
                len1 = len;
            } else {
                self.tree[ptr0 as usize] = current_match;
                ptr0 = pair;
                current_match = self.tree[ptr0 as usize];
                len0 = len;
            }
        }
    }

    /// Skip a run of positions whose distance-one nice match continues through
    /// the same byte value. The scalar skip has already inserted the current
    /// node and established the continuation proof before this is called.
    fn bulk_constant_run(&mut self, encoder: &mut super::LzEncoderData, remaining: i32) -> i32 {
        let nice_len = self.continuation_nice_len;
        if remaining <= 0
            || self.continuation_pos != self.lz_pos
            || self.continuation_delta != 1
            || nice_len < 4
            || encoder.pending_size != 0
        {
            return 0;
        }

        // Keep every move_pos call on the ordinary full-nice path. This also
        // leaves flushing/finishing tails and unavailable positions to scalar
        // handling, where pending_size semantics are preserved.
        let avail = encoder.get_avail();
        let count = remaining
            .min(avail - nice_len)
            .min(MAX_POS - self.lz_pos - 1)
            .min(self.cyclic_size - self.cyclic_pos - 1);
        if count <= 0 {
            return 0;
        }

        // The current node proves `nice_len` equal bytes at distance one.
        // Each further equal byte proves exactly one more adjacent skipped
        // position. `extend_match` scans this bounded run word at a time.
        let matched = extend_match(
            &encoder.buf,
            encoder.read_pos,
            nice_len,
            1,
            nice_len + count,
        ) - nice_len;
        if matched <= 0 {
            return 0;
        }

        let first_child = sh_left(self.cyclic_pos);
        let left = self.tree[first_child as usize];
        let right = self.tree[first_child as usize + 1];
        let first_destination = sh_left(self.cyclic_pos + 1) as usize;
        let end_destination = first_destination + matched as usize * 2;
        for children in self.tree[first_destination..end_destination].chunks_exact_mut(2) {
            children[0] = left;
            children[1] = right;
        }

        self.lz_pos += matched;
        self.cyclic_pos += matched;
        encoder.read_pos += matched;
        self.hash.update_tables(self.lz_pos);
        self.continuation_pos = self.lz_pos;
        matched
    }

    fn skip_one(&mut self, encoder: &mut super::LzEncoderData) {
        let mut nice_len_limit = encoder.nice_len as i32;
        let avail = self.move_pos(encoder);

        if avail < nice_len_limit {
            if avail == 0 {
                return;
            }
            nice_len_limit = avail;
        }

        self.hash.calc_hashes(encoder.read_buffer());
        let current_match = self.hash.get_hash4_pos();
        self.hash.update_tables(self.lz_pos);
        self.skip_tree(encoder, nice_len_limit, current_match);
    }
}

impl MatchFind for Bt4 {
    fn find_matches(&mut self, encoder: &mut super::LzEncoderData, matches: &mut Matches) {
        matches.count = 0;

        let mut match_len_limit = encoder.match_len_max as i32;
        let mut nice_len_limit = encoder.nice_len as i32;
        let avail = self.move_pos(encoder);

        if avail < match_len_limit {
            if avail == 0 {
                return;
            }
            match_len_limit = avail;
            if nice_len_limit > avail {
                nice_len_limit = avail;
            }
        }

        self.hash.calc_hashes(encoder.read_buffer());
        let mut delta2 = self.lz_pos - self.hash.get_hash2_pos();
        let delta3 = self.lz_pos - self.hash.get_hash3_pos();
        let mut current_match = self.hash.get_hash4_pos();
        self.hash.update_tables(self.lz_pos);

        let mut len_best = 0;

        // See if the hash from the first two bytes found a match.
        // The hashing algorithm guarantees that if the first byte
        // matches, also the second byte does, so there's no need to
        // test the second byte.
        if delta2 < self.cyclic_size
            && encoder.get_byte_backward(delta2) == encoder.get_current_byte()
        {
            len_best = 2;
            matches.len[0] = 2;
            matches.dist[0] = delta2 - 1;
            matches.count = 1;
        }

        // See if the hash from the first three bytes found a match that
        // is different from the match possibly found by the two-byte hash.
        // Also, here the hashing algorithm guarantees that if the first byte
        // matches, also the next two bytes do.
        if delta2 != delta3
            && delta3 < self.cyclic_size
            && encoder.get_byte_backward(delta3) == encoder.get_current_byte()
        {
            len_best = 3;
            let count = matches.count as usize;
            matches.dist[count] = delta3 - 1;
            matches.count += 1;
            delta2 = delta3;
        }

        // If a match was found, see how long it is.
        if matches.count > 0 {
            len_best = extend_match(
                &encoder.buf,
                encoder.read_pos,
                len_best,
                delta2,
                match_len_limit,
            );

            let c = matches.count as usize - 1;
            matches.len[c] = len_best as u32;

            // Return if it is long enough (niceLen or reached the end of
            // the dictionary).
            if len_best >= nice_len_limit {
                self.skip_tree(encoder, nice_len_limit, current_match);
                return;
            }
        }

        // Long enough match wasn't found so easily. Look for better matches
        // from the binary tree.
        if len_best < 3 {
            len_best = 3;
        }
        let mut depth = self.depth_limit;

        let mut ptr0 = sh_left(self.cyclic_pos) + 1;
        let mut ptr1 = sh_left(self.cyclic_pos);
        let mut len0 = 0;
        let mut len1 = 0;

        loop {
            let delta = self.lz_pos - current_match;

            // Return if the search depth limit has been reached or
            // if the distance of the potential match exceeds the
            // dictionary size.
            if depth == 0 || delta >= self.cyclic_size {
                self.tree[ptr0 as usize] = 0;
                self.tree[ptr1 as usize] = 0;
                return;
            }
            depth -= 1;

            let pair_selector = self.cyclic_size * ((delta > self.cyclic_pos) as i32);
            let pair = sh_left(self.cyclic_pos - delta + pair_selector);

            let mut len = len0.min(len1);

            len = extend_match(
                encoder.buf.as_slice(),
                encoder.read_pos,
                len,
                delta,
                match_len_limit,
            );

            if len > len_best {
                len_best = len;
                let count = matches.count as usize;
                matches.len[count] = len as _;
                let count = matches.count as usize;
                matches.dist[count] = delta - 1;
                matches.count += 1;

                if len >= nice_len_limit {
                    self.tree[ptr1 as usize] = self.tree[pair as usize];
                    self.tree[ptr0 as usize] = self.tree[pair as usize + 1];
                    return;
                }
            }

            if (encoder.get_byte(len, delta)) < (encoder.get_byte(len, 0)) {
                self.tree[ptr1 as usize] = current_match;
                ptr1 = pair + 1;
                current_match = self.tree[ptr1 as usize];
                len1 = len;
            } else {
                self.tree[ptr0 as usize] = current_match;
                ptr0 = pair;
                current_match = self.tree[ptr0 as usize];
                len0 = len;
            }
        }
    }

    fn skip(&mut self, encoder: &mut super::LzEncoderData, len: usize) {
        let mut len = len as i32;
        len -= self.bulk_constant_run(encoder, len);
        while {
            let n = len > 0;
            len -= 1;
            n
        } {
            self.skip_one(encoder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_state(
        data: &[u8],
        chunks: &[usize],
        finishing: bool,
        bulk_enabled: bool,
    ) -> (Bt4, LzEncoder) {
        let dict_size = 64;
        let nice_len = 16;
        let mut encoder = LzEncoder::new_bt4(dict_size, 0, 0, nice_len, 32, 8);
        encoder.data.buf[..data.len()].copy_from_slice(data);
        encoder.data.write_pos = data.len() as i32;
        encoder.data.finishing = finishing;

        let mut finder = Bt4::new(dict_size, nice_len, 8);
        for _ in 0..2 {
            finder.find_matches(&mut encoder.data, &mut encoder.matches);
        }
        for &chunk in chunks {
            if bulk_enabled {
                MatchFind::skip(&mut finder, &mut encoder.data, chunk);
            } else {
                for _ in 0..chunk {
                    finder.skip_one(&mut encoder.data);
                }
            }
        }
        (finder, encoder)
    }

    fn assert_skip_state_eq(bulk: (&Bt4, &LzEncoder), scalar: (&Bt4, &LzEncoder)) {
        let (bulk_finder, bulk_encoder) = bulk;
        let (scalar_finder, scalar_encoder) = scalar;
        assert_eq!(bulk_finder.tree, scalar_finder.tree);
        assert_eq!(bulk_finder.hash, scalar_finder.hash);
        assert_eq!(bulk_finder.lz_pos, scalar_finder.lz_pos);
        assert_eq!(bulk_finder.cyclic_pos, scalar_finder.cyclic_pos);
        assert_eq!(bulk_finder.continuation_pos, scalar_finder.continuation_pos);
        assert_eq!(
            bulk_finder.continuation_delta,
            scalar_finder.continuation_delta
        );
        assert_eq!(
            bulk_finder.continuation_nice_len,
            scalar_finder.continuation_nice_len
        );
        assert_eq!(bulk_encoder.data.read_pos, scalar_encoder.data.read_pos);
        assert_eq!(
            bulk_encoder.data.pending_size,
            scalar_encoder.data.pending_size
        );
    }

    #[test]
    fn bulk_constant_run_matches_scalar_state_across_wraps_and_tails() {
        let mut changing_runs = vec![b'a'; 8_192];
        changing_runs[2_047..2_049].copy_from_slice(b"bc");
        changing_runs[6_000] = b'z';

        for data in [vec![0; 8_192], changing_runs] {
            for chunks in [&[8_190][..], &[31, 1_024, 2_035, 5_100][..]] {
                for finishing in [false, true] {
                    let bulk = skip_state(&data, chunks, finishing, true);
                    let scalar = skip_state(&data, chunks, finishing, false);
                    assert_skip_state_eq((&bulk.0, &bulk.1), (&scalar.0, &scalar.1));
                }
            }
        }
    }

    #[test]
    fn bulk_constant_run_stops_before_position_normalization() {
        fn run(use_bulk: bool) -> (Bt4, LzEncoder) {
            let mut encoder = LzEncoder::new_bt4(64, 0, 0, 16, 32, 8);
            encoder.data.buf.fill(b'a');
            encoder.data.read_pos = 16;
            encoder.data.write_pos = 128;

            let mut finder = Bt4::new(64, 16, 8);
            finder.lz_pos = MAX_POS - 3;
            finder.cyclic_pos = 8;
            finder.continuation_pos = finder.lz_pos;
            finder.continuation_delta = 1;
            finder.continuation_nice_len = 16;
            finder.hash.calc_hashes(encoder.data.read_buffer());
            finder.hash.update_tables(finder.lz_pos);
            let child = sh_left(finder.cyclic_pos) as usize;
            finder.tree[child] = 11;
            finder.tree[child + 1] = 12;

            if use_bulk {
                MatchFind::skip(&mut finder, &mut encoder.data, 5);
            } else {
                for _ in 0..5 {
                    finder.skip_one(&mut encoder.data);
                }
            }
            (finder, encoder)
        }

        let bulk = run(true);
        let scalar = run(false);
        assert_skip_state_eq((&bulk.0, &bulk.1), (&scalar.0, &scalar.1));
    }

    #[derive(Clone, Copy)]
    struct SkipCase {
        nice_len: usize,
        avail: usize,
        cyclic_pos: i32,
        delta: i32,
        prior_pos: i32,
        prior_delta: i32,
        prior_nice_len: i32,
        mismatch_at_end: bool,
    }

    fn finder_and_encoder(case: SkipCase) -> (Bt4, Bt4, LzEncoder) {
        let SkipCase {
            nice_len,
            avail,
            cyclic_pos,
            delta,
            prior_pos,
            prior_delta,
            prior_nice_len,
            mismatch_at_end,
        } = case;
        let match_len_max = nice_len.max(avail) as u32;
        let mut encoder = LzEncoder::new_bt4(4096, 0, 0, nice_len as u32, match_len_max, 3);
        encoder.data.buf = vec![b'a'; 256];
        encoder.data.read_pos = 64;
        encoder.data.write_pos = encoder.data.read_pos + avail as i32;
        encoder.data.pending_size = u32::from(avail < nice_len);
        if mismatch_at_end {
            encoder.data.buf[(encoder.data.read_pos - delta + nice_len as i32 - 1) as usize] = b'b';
        }

        let lz_pos = 4096 + 1 + 32;
        let mut continued = Bt4::new(4096, nice_len as u32, 3);
        let mut baseline = Bt4::new(4096, nice_len as u32, 3);
        for finder in [&mut continued, &mut baseline] {
            finder.cyclic_pos = cyclic_pos;
            finder.lz_pos = lz_pos;
            let pair_selector = finder.cyclic_size * ((delta > cyclic_pos) as i32);
            let pair = sh_left(cyclic_pos - delta + pair_selector) as usize;
            finder.tree[pair] = 23;
            finder.tree[pair + 1] = 24;
        }
        continued.continuation_pos = prior_pos;
        continued.continuation_delta = prior_delta;
        continued.continuation_nice_len = prior_nice_len;
        (continued, baseline, encoder)
    }

    fn assert_same_tree_after_skip(case: SkipCase) {
        let (mut continued, mut baseline, mut encoder) = finder_and_encoder(case);
        let current_match = continued.lz_pos - case.delta;
        let nice_len_limit = case.nice_len.min(case.avail) as i32;
        continued.skip_tree(&mut encoder.data, nice_len_limit, current_match);
        baseline.skip_tree(&mut encoder.data, nice_len_limit, current_match);

        assert_eq!(continued.tree, baseline.tree);
    }

    #[test]
    fn continuation_matches_baseline_tree_splice_and_rejects_stale_state() {
        for (nice_len, avail, cyclic_pos, delta) in [(8, 12, 0, 3), (32, 40, 5, 3)] {
            assert_same_tree_after_skip(SkipCase {
                nice_len,
                avail,
                cyclic_pos,
                delta,
                prior_pos: 4096 + 1 + 31,
                prior_delta: delta,
                prior_nice_len: nice_len as i32,
                mismatch_at_end: false,
            });
        }

        let lz_pos = 4096 + 1 + 32;
        for (avail, prior_pos, prior_delta, prior_nice, mismatch) in [
            (12, lz_pos - 2, 3, 8, false), // not adjacent
            (12, lz_pos - 1, 4, 8, false), // changed distance
            (12, lz_pos - 1, 3, 7, false), // changed nice length
            (12, lz_pos - 1, 3, 8, true),  // new trailing byte differs
            (7, lz_pos - 1, 3, 8, false),  // partial input
        ] {
            assert_same_tree_after_skip(SkipCase {
                nice_len: 8,
                avail,
                cyclic_pos: 0,
                delta: 3,
                prior_pos,
                prior_delta,
                prior_nice_len: prior_nice,
                mismatch_at_end: mismatch,
            });
        }
    }

    #[test]
    fn normalization_invalidates_continuation() {
        let mut encoder = LzEncoder::new_bt4(4096, 0, 0, 8, 16, 1);
        encoder.data.buf.fill(b'a');
        encoder.data.read_pos = 8;
        encoder.data.write_pos = 16;
        encoder.data.finishing = true;
        let mut finder = Bt4::new(4096, 8, 1);
        finder.lz_pos = MAX_POS - 1;
        finder.continuation_pos = MAX_POS - 2;
        finder.continuation_delta = 3;
        finder.continuation_nice_len = 8;

        assert_ne!(finder.move_pos(&mut encoder.data), 0);

        assert_eq!(finder.continuation_pos, -2);
        let mut baseline = Bt4::new(4096, 8, 1);
        baseline.lz_pos = finder.lz_pos;
        baseline.cyclic_pos = finder.cyclic_pos;
        baseline.tree.clone_from(&finder.tree);
        let current_match = finder.lz_pos - 3;
        finder.skip_tree(&mut encoder.data, 8, current_match);
        baseline.skip_tree(&mut encoder.data, 8, current_match);
        assert_eq!(finder.tree, baseline.tree);
    }

    /// Reach a third node after both bounds establish a nonzero common prefix.
    #[test]
    fn skip_preserves_descendant_links_with_inherited_prefix() {
        let mut encoder = LzEncoder::new_bt4(4096, 0, 0, 32, 273, 3);
        let mut finder = Bt4::new(4096, 32, 3);
        for prefix in [1usize, 7, 8, 9] {
            let limit = prefix + 17;
            for first_is_lower in [false, true] {
                for third_byte in *b"lmn" {
                    encoder.data.buf = vec![b'm'; 4 * limit];
                    encoder.data.read_pos = (3 * limit) as i32;
                    // The first two nodes lie on opposite sides of the new key.
                    // The third traversal inherits min(prefix, prefix + 2).
                    encoder.data.buf[2 * limit + prefix] = if first_is_lower { b'l' } else { b'n' };
                    encoder.data.buf[limit + prefix + 2] = if first_is_lower { b'n' } else { b'l' };
                    encoder.data.buf[prefix + 3] = third_byte;

                    finder.tree.fill(0);
                    finder.cyclic_pos = 0;
                    finder.lz_pos = finder.cyclic_size;
                    let nodes = [1, 2, 3].map(|distance| finder.lz_pos - (distance * limit) as i32);
                    let pairs = nodes.map(|node| node as usize * 2);
                    let first_child = pairs[0] + usize::from(first_is_lower);
                    let second_child = pairs[1] + usize::from(!first_is_lower);
                    finder.tree[first_child] = nodes[1];
                    finder.tree[second_child] = nodes[2];
                    finder.tree[pairs[2]] = 21;
                    finder.tree[pairs[2] + 1] = 22;

                    let mut expected = finder.tree.clone();
                    expected[usize::from(!first_is_lower)] = nodes[0];
                    expected[usize::from(first_is_lower)] = nodes[1];
                    let (lower_link, upper_link) = if first_is_lower {
                        (first_child, second_child)
                    } else {
                        (second_child, first_child)
                    };
                    match third_byte {
                        b'm' => {
                            // A full match adopts both children of the third node.
                            expected[lower_link] = 21;
                            expected[upper_link] = 22;
                        }
                        b'l' => {
                            expected[lower_link] = nodes[2];
                            expected[upper_link] = 0;
                            expected[pairs[2] + 1] = 0;
                        }
                        b'n' => {
                            expected[upper_link] = nodes[2];
                            expected[lower_link] = 0;
                            expected[pairs[2]] = 0;
                        }
                        _ => unreachable!(),
                    }
                    finder.skip_tree(&mut encoder.data, limit as i32, nodes[0]);
                    assert_eq!(
                        finder.tree, expected,
                        "prefix={prefix}, first_is_lower={first_is_lower}, third_byte={third_byte}"
                    );
                }
            }
        }
    }

    #[test]
    fn skip_preserves_tree_links_at_word_and_nice_length_boundaries() {
        let mut encoder = LzEncoder::new_bt4(4096, 0, 0, 32, 273, 1);
        let mut finder = Bt4::new(4096, 32, 1);
        for limit in [4, 7, 8, 9, 31, 32, 33, 273] {
            for mismatch in 0..=limit {
                for previous_byte in *b"ln" {
                    // End the buffer exactly at the comparison limit. Position
                    // zero in the cyclic tree forces the prior node to wrap.
                    encoder.data.buf = vec![b'm'; 2 * limit];
                    encoder.data.read_pos = limit as i32;
                    if mismatch < limit {
                        encoder.data.buf[mismatch] = previous_byte;
                    }
                    finder.tree.fill(0);
                    finder.cyclic_pos = 0;
                    finder.lz_pos = finder.cyclic_size;
                    let current_match = finder.lz_pos - limit as i32;
                    let pair = current_match as usize * 2;
                    finder.tree[pair] = 21;
                    finder.tree[pair + 1] = 22;
                    finder.skip_tree(&mut encoder.data, limit as i32, current_match);
                    let expected = if mismatch == limit {
                        [21, 22]
                    } else if previous_byte < b'm' {
                        [current_match, 0]
                    } else {
                        [0, current_match]
                    };
                    assert_eq!(
                        finder.tree[..2],
                        expected,
                        "limit={limit}, mismatch={mismatch}, previous={previous_byte}"
                    );
                }
            }
        }
    }
}
