use std::{
    collections::VecDeque,
    io,
    panic::AssertUnwindSafe,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::{self, JoinHandle},
};

use super::lz_encoder::{LzEncoderData, MatchFind, MatchFinders, Matches};

pub(super) const INPUT_SIZE: usize = 64 * 1024;
pub(super) const POSITIONS: usize = 4096;
const RESULT_POSITIONS: usize = 8192;

enum Input {
    Bytes(Vec<u8>),
    Finish,
}

#[derive(Default)]
struct QueueState {
    inputs: VecDeque<Input>,
    input_bytes: usize,
    results: VecDeque<MatchBatch>,
    result_positions: usize,
    recycled: Vec<MatchBatch>,
    stopped: bool,
}

struct Queues {
    state: Mutex<QueueState>,
    input_ready: Condvar,
    results_ready: Condvar,
    result_space: Condvar,
    input_capacity: usize,
}

impl Queues {
    fn lock(&self) -> MutexGuard<'_, QueueState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn stop(&self) {
        self.lock().stopped = true;
        self.input_ready.notify_all();
        self.results_ready.notify_all();
        self.result_space.notify_all();
    }

    fn input(&self) -> Option<Input> {
        let state = self
            .input_ready
            .wait_while(self.lock(), |state| {
                state.inputs.is_empty() && !state.stopped
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut state = state;
        if state.stopped {
            return None;
        }
        let input = state.inputs.pop_front()?;
        if let Input::Bytes(bytes) = &input {
            state.input_bytes -= bytes.len();
        }
        Some(input)
    }

    fn publish(&self, batch: MatchBatch) -> Option<MatchBatch> {
        let mut state = self
            .result_space
            .wait_while(self.lock(), |state| {
                state.result_positions + batch.len() > RESULT_POSITIONS && !state.stopped
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.stopped {
            return None;
        }
        let wake = state.results.is_empty();
        state.result_positions += batch.len();
        state.results.push_back(batch);
        let recycled = state.recycled.pop().unwrap_or_default();
        drop(state);
        if wake {
            self.results_ready.notify_one();
        }
        Some(recycled)
    }
}

struct WorkerExit(Arc<Queues>);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.stop();
    }
}

#[derive(Default)]
struct MatchBatch {
    offsets: Vec<u32>,
    pairs: Vec<(u32, i32)>,
    position: usize,
}

impl MatchBatch {
    fn prepare(&mut self, positions: usize) {
        // Only reuse full slabs for full slabs. A large allocation must not
        // become an arbitrarily small queued result after fragmented input.
        if positions != POSITIONS && self.offsets.capacity() > positions + 1 {
            *self = Self::default();
        }
        self.position = 0;
        self.offsets.clear();
        self.offsets.reserve_exact(positions + 1);
        self.offsets.push(0);
        self.pairs.clear();
        self.pairs.reserve_exact(positions);
    }

    fn push(&mut self, matches: &Matches, pair_limit: usize) {
        let count = matches.count as usize;
        if count == 1 {
            if self.pairs.len() == self.pairs.capacity() {
                self.grow(1, pair_limit);
            }
            self.pairs.push((matches.len[0], matches.dist[0]));
        } else if count != 0 {
            if count > self.pairs.capacity() - self.pairs.len() {
                self.grow(count, pair_limit);
            }
            self.pairs.extend(
                matches.len[..count]
                    .iter()
                    .copied()
                    .zip(matches.dist[..count].iter().copied()),
            );
        }
        self.offsets.push(self.pairs.len() as u32);
    }

    fn grow(&mut self, count: usize, pair_limit: usize) {
        let required = self.pairs.len() + count;
        // Cap geometric growth at the bound used by the memory estimate.
        let capacity = required.max((2 * self.pairs.capacity()).min(pair_limit));
        self.pairs.reserve_exact(capacity - self.pairs.len());
    }

    fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    fn exhausted(&self) -> bool {
        self.position == self.len()
    }

    fn next(&mut self, matches: Option<&mut Matches>) {
        let start = self.offsets[self.position] as usize;
        let end = self.offsets[self.position + 1] as usize;
        if let Some(matches) = matches {
            matches.count = (end - start) as u32;
            for (index, &(length, distance)) in self.pairs[start..end].iter().enumerate() {
                matches.len[index] = length;
                matches.dist[index] = distance;
            }
        }
        self.position += 1;
    }
}

pub(super) struct MatchPipeline {
    queues: Arc<Queues>,
    // Shared state is mutex-protected, and the handle is taken before joining.
    // Assert only the handle's unwind safety to preserve the writers' auto traits.
    worker: Option<AssertUnwindSafe<JoinHandle<()>>>,
    batch: MatchBatch,
}

impl MatchPipeline {
    pub(super) fn new(mut data: LzEncoderData, mut finder: MatchFinders) -> io::Result<Self> {
        data.match_len_max = data.nice_len;
        let queues = Arc::new(Queues {
            state: Mutex::new(QueueState::default()),
            input_ready: Condvar::new(),
            results_ready: Condvar::new(),
            result_space: Condvar::new(),
            input_capacity: data.buf.len(),
        });
        let worker_queues = Arc::clone(&queues);
        let worker = thread::Builder::new()
            .name("lzma-match-finder".into())
            .spawn(move || {
                let _exit = WorkerExit(Arc::clone(&worker_queues));
                let mut matches = Matches::new(data.nice_len as usize - 1);
                let mut repeat = super::bt4::RepeatMatch::default();
                let mut batch = MatchBatch::default();
                for input in std::iter::from_fn(|| worker_queues.input()) {
                    match input {
                        Input::Bytes(bytes) => {
                            let used = data.fill_window(&bytes);
                            assert_eq!(
                                used,
                                bytes.len(),
                                "matcher window must admit encoder input"
                            );
                        }
                        Input::Finish => data.set_finishing(),
                    }
                    let required = if data.finishing {
                        4
                    } else {
                        data.nice_len as i32
                    };
                    let positions = (data.write_pos - data.read_pos - required).max(0) as usize;
                    for start in (0..positions).step_by(POSITIONS) {
                        let count = (positions - start).min(POSITIONS);
                        batch.prepare(count);
                        let pair_limit = count * (data.nice_len as usize - 1);
                        for _ in 0..count {
                            match &mut finder {
                                MatchFinders::Bt4(finder) => {
                                    finder.find_matches_cached(&mut data, &mut matches, &mut repeat)
                                }
                                MatchFinders::Hc4(finder) => {
                                    finder.find_matches(&mut data, &mut matches)
                                }
                            }
                            batch.push(&matches, pair_limit);
                        }
                        batch = match worker_queues.publish(batch) {
                            Some(recycled) => recycled,
                            None => return,
                        };
                    }
                    if data.finishing {
                        return;
                    }
                }
            })?;
        Ok(Self {
            queues,
            worker: Some(AssertUnwindSafe(worker)),
            batch: MatchBatch::default(),
        })
    }

    pub(super) fn overhead(window_size: u64, nice_len: u64) -> u64 {
        // A mirrored window, geometrically allocated input packets, a producer
        // and consumer slab, and position-bounded queued results. Tiny writes
        // may produce one-position slabs, so include their metadata as well.
        3 * window_size
            + 2 * INPUT_SIZE as u64
            + (RESULT_POSITIONS as u64 + 4 * POSITIONS as u64)
                * (std::mem::size_of::<u32>() as u64
                    + nice_len.saturating_sub(1) * std::mem::size_of::<(u32, i32)>() as u64)
            + (RESULT_POSITIONS as u64 + 4)
                * (std::mem::size_of::<MatchBatch>() + std::mem::size_of::<u32>()) as u64
            + (window_size / INPUT_SIZE as u64 + 2) * std::mem::size_of::<Input>() as u64
    }

    pub(super) fn input(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        let queues = &self.queues;
        let mut state = queues.lock();
        if state.stopped {
            drop(state);
            return Err(self.disconnected());
        }
        if bytes.len() > queues.input_capacity - state.input_bytes {
            return Err(io::Error::other("match finder input window exceeded"));
        }
        let wake = state.inputs.is_empty();
        state.input_bytes += bytes.len();
        if let Some(Input::Bytes(tail)) = state.inputs.back_mut() {
            let count = bytes.len().min(INPUT_SIZE - tail.len());
            tail.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
        }
        state.inputs.extend(
            bytes
                .chunks(INPUT_SIZE)
                .map(|bytes| Input::Bytes(bytes.to_vec())),
        );
        drop(state);
        if wake {
            queues.input_ready.notify_one();
        }
        Ok(())
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        // Flush changes only the encoder read limit. The matcher already
        // publishes every position with enough lookahead; its short tail is
        // replayed when more input arrives, just as in the local finder.
        if self.queues.lock().stopped {
            return Err(self.disconnected());
        }
        Ok(())
    }

    pub(super) fn finish(&mut self) -> io::Result<()> {
        let queues = Arc::clone(&self.queues);
        let mut state = queues.lock();
        if state.stopped {
            drop(state);
            return Err(self.disconnected());
        }
        state.inputs.push_back(Input::Finish);
        queues.input_ready.notify_one();
        Ok(())
    }

    fn next(&mut self, matches: Option<&mut Matches>) -> io::Result<()> {
        if self.batch.exhausted() {
            self.advance_batch()?;
        }
        self.batch.next(matches);
        Ok(())
    }

    fn advance_batch(&mut self) -> io::Result<()> {
        let queues = &self.queues;
        let mut state = queues
            .results_ready
            .wait_while(queues.lock(), |state| {
                state.results.is_empty() && !state.stopped
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let batch = state.results.pop_front();
        if let Some(batch) = batch {
            state.result_positions -= batch.len();
            let old = std::mem::replace(&mut self.batch, batch);
            if old.len() == POSITIONS && state.recycled.len() < 2 {
                state.recycled.push(old);
            }
            drop(state);
            queues.result_space.notify_one();
        } else {
            drop(state);
            return Err(self.disconnected());
        }
        Ok(())
    }

    #[inline(never)]
    pub(super) fn find_matches(
        &mut self,
        data: &mut LzEncoderData,
        matches: &mut Matches,
    ) -> io::Result<()> {
        matches.count = 0;
        if data.move_pos(data.nice_len as i32, 4) == 0 {
            return Ok(());
        }
        self.next(Some(matches))?;
        if let Some(index) = (matches.count as usize).checked_sub(1) {
            if matches.len[index] == data.nice_len {
                matches.len[index] = data.get_match_len(
                    matches.dist[index],
                    data.get_avail().min(data.match_len_max as i32),
                ) as u32;
            }
        }
        Ok(())
    }

    #[inline(never)]
    pub(super) fn skip(&mut self, data: &mut LzEncoderData, count: usize) -> io::Result<()> {
        let required = if data.finishing {
            4
        } else {
            data.nice_len as i32
        };
        let mut ready = ((data.write_pos - data.read_pos - required).max(0) as usize).min(count);
        data.read_pos += count as i32;
        data.pending_size += (count - ready) as u32;
        while ready != 0 {
            if self.batch.exhausted() {
                self.advance_batch()?;
            }
            let n = ready.min(self.batch.len() - self.batch.position);
            self.batch.position += n;
            ready -= n;
        }
        Ok(())
    }

    fn disconnected(&mut self) -> io::Error {
        self.queues.stop();
        match self.worker.take().map(|worker| worker.0.join()) {
            Some(Err(_)) => io::Error::other("match finder worker panicked"),
            _ => io::Error::other("match finder worker disconnected"),
        }
    }
}

impl Drop for MatchPipeline {
    fn drop(&mut self) {
        self.queues.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.0.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{super::LzEncoder, *};

    #[test]
    fn skipping_results_keeps_the_next_match_at_its_offset() {
        let mut batch = MatchBatch::default();
        batch.prepare(5);
        let mut matches = Matches::new(31);
        for count in [2, 0, 3, 1, 2] {
            matches.count = count;
            for i in 0..count as usize {
                matches.len[i] = count + i as u32;
                matches.dist[i] = i as i32;
            }
            batch.push(&matches, 5 * 31);
        }
        batch.position += 3;
        batch.next(Some(&mut matches));
        assert_eq!(matches.count, 1);
        assert_eq!(matches.len[0], 1);
        assert_eq!(matches.dist[0], 0);
        batch.next(None);
        assert!(batch.exhausted());
    }

    #[test]
    fn recycled_slabs_do_not_inflate_fragmented_result_memory() {
        let mut batch = MatchBatch::default();
        batch.prepare(POSITIONS);
        let offsets = batch.offsets.as_ptr();
        let pairs = batch.pairs.as_ptr();
        batch.prepare(POSITIONS);
        assert_eq!(batch.offsets.as_ptr(), offsets);
        assert_eq!(batch.pairs.as_ptr(), pairs);
        batch.prepare(1);
        assert_eq!(batch.offsets.capacity(), 2);
        assert_eq!(batch.pairs.capacity(), 1);
    }

    #[test]
    fn result_storage_grows_with_matches_without_exceeding_the_bound() {
        let mut batch = MatchBatch::default();
        batch.prepare(3);
        assert_eq!(batch.pairs.capacity(), 3);
        let mut matches = Matches::new(63);
        matches.count = 63;
        for i in 0..63 {
            matches.len[i] = i as u32 + 2;
            matches.dist[i] = i as i32;
        }
        for _ in 0..3 {
            batch.push(&matches, 3 * 63);
            assert!(batch.pairs.capacity() <= 3 * 63);
        }
        assert_eq!(batch.pairs.len(), 3 * 63);
        assert_eq!(batch.offsets, [0, 63, 126, 189]);
        matches.count = 0;
        for _ in 0..3 {
            batch.next(Some(&mut matches));
            assert_eq!(matches.count, 63);
            assert_eq!(matches.len[62], 64);
            assert_eq!(matches.dist[62], 62);
        }
        let pairs = batch.pairs.as_ptr();
        batch.prepare(3);
        assert_eq!(batch.pairs.as_ptr(), pairs);
        assert_eq!(batch.pairs.capacity(), 3 * 63);
    }

    fn pipeline() -> MatchPipeline {
        let encoder = LzEncoder::new_bt4(64 * 1024, 4096, 4096, 32, 273, 32);
        MatchPipeline::new(
            encoder.data,
            MatchFinders::Bt4(super::super::bt4::Bt4::new(64 * 1024, 32, 32)),
        )
        .unwrap()
    }

    #[test]
    fn dropping_with_queued_results_joins_the_worker() {
        let mut pipeline = pipeline();
        pipeline.input(&vec![0; 64 * 1024]).unwrap();
        pipeline.next(None).unwrap();
        drop(pipeline);
    }

    #[test]
    fn oversized_input_is_rejected_before_queueing() {
        let mut pipeline = pipeline();
        let capacity = pipeline.queues.input_capacity;
        assert!(pipeline.input(&vec![0; capacity + 1]).is_err());
        assert_eq!(pipeline.queues.lock().input_bytes, 0);
    }

    #[test]
    fn worker_panic_wakes_the_reader_and_becomes_an_io_error() {
        let mut pipeline = pipeline();
        pipeline.queues.stop();
        pipeline.worker.take().unwrap().0.join().unwrap();
        pipeline.queues.lock().stopped = false;
        let queues = Arc::clone(&pipeline.queues);
        pipeline.worker = Some(AssertUnwindSafe(thread::spawn(move || {
            let _exit = WorkerExit(queues);
            panic!("injected worker panic");
        })));
        let error = pipeline.next(None).unwrap_err();
        assert_eq!(error.to_string(), "match finder worker panicked");
        assert!(pipeline.worker.is_none());
    }
}
