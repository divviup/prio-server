use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};

/// A queue of jobs that may be shared across multiple consumer threads. The
/// producer must have the entire list of jobs in hand when creating the
/// WorkQueue; adding new jobs to an existing WorkQueue is not supported.
///
/// A note on thread safety: this struct stores important data in Arc+Mutex so
/// it may be clone()d and shared across threads liberally, allowing multiple
/// consumers to safely and efficiently dequeue jobs and send results to a
/// single WorkQueue.
#[derive(Debug, Clone)]
pub(crate) struct WorkQueue<T, R> {
    // The list of jobs, wrapped in an std::sync::Mutex and std::sync::Arc for
    // thread safety. Most work queue implementations would use an
    // std::collections::VecDeque to allow new jobs to be pushed to the back of
    // the queue, but since we require clients to have all the jobs in hand
    // when they call WorkQueue::new(), we can just use a plain Vec and save an
    // allocation and copy.
    jobs: Arc<Mutex<Vec<T>>>,
    results: Arc<Mutex<Vec<R>>>,
}

impl<T, R> WorkQueue<T, R> {
    /// Creates a new WorkQueue from the provided list of jobs
    pub(crate) fn new(jobs: Vec<T>) -> Self {
        WorkQueue {
            jobs: Arc::new(Mutex::new(jobs)),
            results: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns a job from the queue, or None if the queue is empty.
    ///
    /// # Panics
    ///
    /// Panics if the mutex protecting the job queue is poisoned.
    pub(crate) fn dequeue_job(&mut self) -> Option<T> {
        self.jobs.lock().unwrap().pop()
    }

    /// Send job results to the queue. Clients need not call this 1:1 with
    /// dequeue_job, if they can reasonably represent there results of multiple
    /// jobs in one R.
    ///
    /// # Panics
    ///
    /// Panics if the mutex protecting the results is poisoned.
    pub(crate) fn send_results(&mut self, result: R) {
        self.results.lock().unwrap().push(result)
    }

    /// Get the results from this work queue's completed jobs, consuming the
    /// work queue. Callers should ensure that all other references to the
    /// WorkQueue have been dropped before calling this method (i.e., make sure
    /// any clones of this instance have been dropped).
    ///
    /// # Errors
    ///
    /// Returns an error if there are jobs left in the queue, or if the Arc
    /// protecting the results cannot be unwrapped because there are outstanding
    /// strong references.
    ///
    /// # Panics
    ///
    /// Panics if the Mutex protecting the results is poisoned.
    pub(crate) fn results(self) -> Result<Vec<R>> {
        if !self.jobs.lock().unwrap().is_empty() {
            return Err(anyhow!("cannot get results before all jobs are dequeued"));
        }
        let mutex = Arc::try_unwrap(self.results)
            .map_err(|_| anyhow!("failed to unwrap Arc (outstanding strong reference to work queue in a worker thread?)"))?;
        Ok(mutex.into_inner().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use std::thread::{self, JoinHandle};

    #[test]
    fn dequeue_jobs() {
        let work_queue: WorkQueue<Vec<u32>, u32> =
            WorkQueue::new(vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]]);
        let mut thread_handles: Vec<JoinHandle<()>> = Vec::new();

        for _ in 0..2 {
            let mut queue_clone = work_queue.clone();
            let handle = thread::spawn(move || loop {
                match queue_clone.dequeue_job() {
                    Some(job) => queue_clone.send_results(job.iter().sum()),
                    None => break (),
                }
            });
            thread_handles.push(handle);
        }

        for handle in thread_handles {
            handle.join().unwrap();
        }

        let results = work_queue.results().unwrap();

        assert_eq!(results.iter().sum::<u32>(), 45);
    }

    #[test]
    fn dequeue_error() {
        let work_queue: WorkQueue<std::result::Result<(), &'static str>, ()> =
            WorkQueue::new(vec![Ok(()), Err("fake error")]);

        let mut queue_clone = work_queue.clone();
        let thread_handle: JoinHandle<Result<(), &'static str>> = thread::spawn(move || loop {
            assert_matches!(queue_clone.dequeue_job(), Some(job) => {
                match job {
                    Ok(()) => queue_clone.send_results(()),
                    Err(e) => break Err(e),
                }
            });
        });

        assert_matches!(thread_handle.join(), Ok(Err("fake error")));
    }
}
