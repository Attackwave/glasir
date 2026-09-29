//! Small, deterministic parallel helpers built only on the standard library.
//!
//! Glasir has a few CPU-bound passes where bounded parallelism matters, but it
//! does not need a general work-stealing runtime.  These helpers split an input
//! into at most eight contiguous ranges and join them in input order.  That
//! makes scheduling an implementation detail rather than a source of changed
//! node ids or ranking results.

const MAX_WORKERS: usize = 8;

fn workers(work: usize) -> usize {
    if work < 2 {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(MAX_WORKERS)
        .min(work)
}

/// Maps a slice concurrently and returns results in precisely input order.
pub fn map_ordered<T, R>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    let count = workers(items.len());
    let chunk = items.len().div_ceil(count);
    std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for (part, slice) in items.chunks(chunk).enumerate() {
            let f = &f;
            jobs.push(scope.spawn(move || {
                slice
                    .iter()
                    .map(f)
                    .collect::<Vec<R>>()
                    .into_iter()
                    .map(|value| (part, value))
                    .collect::<Vec<_>>()
            }));
        }
        let mut parts = Vec::with_capacity(jobs.len());
        for job in jobs {
            parts.push(job.join().expect("parallel worker panicked"));
        }
        parts.sort_by_key(|part| part.first().map_or(usize::MAX, |(index, _)| *index));
        parts
            .into_iter()
            .flatten()
            .map(|(_, value)| value)
            .collect()
    })
}

/// Maps `0..len` concurrently and returns results in ascending index order.
pub fn map_range<R>(len: usize, f: impl Fn(usize) -> R + Sync) -> Vec<R>
where
    R: Send,
{
    let count = workers(len);
    let chunk = len.div_ceil(count);
    std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for start in (0..len).step_by(chunk.max(1)) {
            let end = (start + chunk).min(len);
            let f = &f;
            jobs.push(scope.spawn(move || (start..end).map(f).collect::<Vec<R>>()));
        }
        jobs.into_iter()
            .flat_map(|job| job.join().expect("parallel worker panicked"))
            .collect()
    })
}

/// Calls `f` for disjoint contiguous groups of `unit` elements.
pub fn for_each_unit_mut<T>(items: &mut [T], unit: usize, f: impl Fn(usize, &mut [T]) + Sync)
where
    T: Send,
{
    assert!(unit > 0 && items.len().is_multiple_of(unit));
    let units = items.len() / unit;
    let chunk_units = units.div_ceil(workers(units));
    std::thread::scope(|scope| {
        for (part, slice) in items.chunks_mut(chunk_units * unit).enumerate() {
            let f = &f;
            scope.spawn(move || f(part * chunk_units, slice));
        }
    });
}
