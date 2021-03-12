mod pubsub;
mod sqs;

use anyhow::Result;
use dyn_clone::{clone_trait_object, DynClone};
use log::error;
use serde::Deserialize;
use std::{
    fmt,
    fmt::{Debug, Display},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub use pubsub::GcpPubSubTaskQueue;
pub use sqs::AwsSqsTaskQueue;

/// A queue of tasks to be executed
pub trait TaskQueue<T: Task>: Debug + DynClone + Send + Sync + 'static {
    /// Get a task to execute. If a task to run is found, returns Ok(Some(T)).
    /// If a task is successfully checked for but there is no work available,
    /// returns Ok(None). Returns Err(e) if something goes wrong.
    /// Once the task has been successfully completed, the TaskHandle should be
    /// passed to acknowledge_task to permanently remove the task. If
    /// acknowledge_task is never called, the task will eventually be
    /// re-delivered via dequeue().
    fn dequeue(&mut self) -> Result<Option<TaskHandle<T>>>;

    /// Signal to the task queue that the task has been handled and should be
    /// removed from the queue.
    fn acknowledge_task(&mut self, handle: TaskHandle<T>) -> Result<()>;

    /// Signal to the task queue that the task was not handled and should be
    /// retried later.
    fn nacknowledge_task(&mut self, handle: TaskHandle<T>) -> Result<()>;

    /// Signal to the task queue that more time is needed to handle the task.
    fn extend_task_deadline(&mut self, handle: &TaskHandle<T>, increment: &Duration) -> Result<()>;
}

clone_trait_object!(<T: Task> TaskQueue<T>);

impl<T: Task> TaskQueue<T> for Box<dyn TaskQueue<T>> {
    fn dequeue(&mut self) -> Result<Option<TaskHandle<T>>> {
        (**self).dequeue()
    }

    fn acknowledge_task(&mut self, handle: TaskHandle<T>) -> Result<()> {
        (**self).acknowledge_task(handle)
    }

    fn nacknowledge_task(&mut self, handle: TaskHandle<T>) -> Result<()> {
        (**self).nacknowledge_task(handle)
    }

    fn extend_task_deadline(&mut self, handle: &TaskHandle<T>, increment: &Duration) -> Result<()> {
        (**self).extend_task_deadline(handle, increment)
    }
}

/// Represents a task that can be assigned to a worker
pub trait Task:
    Debug + Display + PartialEq + Clone + Send + Sized + Sync + serde::de::DeserializeOwned + 'static
{
}

/// Represents an intake batch task to be executed
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct IntakeBatchTask {
    /// The trace identifier for the intake
    /// TODO: https://github.com/abetterinternet/prio-server/issues/452
    pub trace_id: Option<Uuid>,
    /// The identifier for the aggregation
    pub aggregation_id: String,
    /// The identifier of the batch, typically a UUID
    pub batch_id: String,
    /// The UTC timestamp on the batch, with minute precision, formatted like
    /// "2006/01/02/15/04"
    pub date: String,
}

impl Task for IntakeBatchTask {}

impl Display for IntakeBatchTask {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(id) = self.trace_id {
            write!(f, "trace ID: {}\n", id)?;
        }
        write!(
            f,
            "aggregation ID: {}\nbatch ID: {}\ndate: {}",
            self.aggregation_id, self.batch_id, self.date
        )
    }
}

/// Represents an aggregation task to be executed
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct AggregationTask {
    /// The trace identifier for the aggregation
    /// TODO: https://github.com/abetterinternet/prio-server/issues/452
    pub trace_id: Option<Uuid>,
    /// The identifier for the aggregation
    pub aggregation_id: String,
    /// The start of the range of time covered by the aggregation in UTC, with
    /// minute precision, formatted like "2006/01/02/15/04"
    pub aggregation_start: String,
    /// The end of the range of time covered by the aggregation in UTC, with
    /// minute precision, formatted like "2006/01/02/15/04"
    pub aggregation_end: String,
    // The list of batches aggregated by this task
    pub batches: Vec<Batch>,
}

impl Task for AggregationTask {}

impl Display for AggregationTask {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(id) = self.trace_id {
            write!(f, "trace ID: {}\n", id)?;
        }
        write!(
            f,
            "aggregation ID: {}\naggregation start: {}\naggregation end: {}\nnumber of batches: {}",
            self.aggregation_id,
            self.aggregation_start,
            self.aggregation_end,
            self.batches.len()
        )
    }
}

/// Represents a batch included in an aggregation
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Batch {
    /// The identifier of the batch. Typically a UUID.
    pub id: String,
    /// The timestamp on the batch, in UTC, with minute precision, formatted
    /// like "2006/01/02/15/04".
    pub time: String,
}

/// A TaskHandle wraps a Task along with whatever metadata is needed by a
/// TaskQueue implementation
#[derive(Clone, Debug, PartialEq)]
pub struct TaskHandle<T: Task> {
    /// The acknowledgment ID for the task
    acknowledgment_id: String,
    /// The task
    pub task: T,
}

impl<T: Task> Display for TaskHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ack ID: {}\ntask: {}", self.acknowledgment_id, self.task)
    }
}

/// Watchdogs allow workers processing long-running tasks to indicate that they
/// are still making progress.
pub trait Watchdog: Clone + Send + Sync + 'static {
    /// Indicate to the watchdog that progress is being made on some work.
    fn send_heartbeat(&self);
}

/// A Watchdog that does nothing.
#[derive(Clone)]
pub struct NoOpWatchdog {}

impl Watchdog for NoOpWatchdog {
    fn send_heartbeat(&self) {}
}

/// An object that returns some perception of the current time.
pub trait Clock: Clone + Send + Sync + 'static {
    fn now(&self) -> Instant;
}

/// A Clock that returns the current time according to std::time::Instant::now.
#[derive(Clone)]
pub struct RealClock {}

impl Clock for RealClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A watchdog that periodically extends the deadline on a TaskHandle in a
/// TaskQueue.
#[derive(Debug, Clone)]
pub struct TaskQueueWatchdog<T, Q, C>
where
    T: Task,
    Q: TaskQueue<T> + Clone,
    C: Clock,
{
    task_handle: TaskHandle<T>,
    previous_deadline_extension: Arc<Mutex<Instant>>,
    task_queue: Arc<Mutex<Q>>,
    clock: C,
}

impl<T, Q> TaskQueueWatchdog<T, Q, RealClock>
where
    T: Task,
    Q: TaskQueue<T> + Clone,
{
    /// Create a new watchdog to periodically extend the deadline on the
    /// provided task handle, in the provided task queue.
    pub fn new(task_handle: TaskHandle<T>, task_queue: Arc<Mutex<Q>>) -> Self {
        Self::new_with_previous_deadline_extension(
            task_handle,
            &Instant::now(),
            task_queue,
            RealClock {},
        )
    }
}

impl<T, Q, C> TaskQueueWatchdog<T, Q, C>
where
    T: Task,
    Q: TaskQueue<T> + Clone,
    C: Clock,
{
    // Allows construction of a TaskQueueWatchdog with a client controlled Clock
    // for testing.
    fn new_with_previous_deadline_extension(
        task_handle: TaskHandle<T>,
        instant: &Instant,
        task_queue: Arc<Mutex<Q>>,
        clock: C,
    ) -> Self {
        Self {
            task_handle,
            previous_deadline_extension: Arc::new(Mutex::new(instant.clone())),
            task_queue,
            clock,
        }
    }
}

impl<T, Q, C> Watchdog for TaskQueueWatchdog<T, Q, C>
where
    T: Task,
    Q: TaskQueue<T> + Clone,
    C: Clock,
{
    fn send_heartbeat(&self) {
        // We assume that 10 minutes is a reasonable deadline increment
        // regardless of queue implementation or what the task is (it is also
        // maximum deadline extension allowed by GCP PubSub). In the future this
        // could be a tunable parameter on facilitator.
        let deadline_increment = Duration::from_secs(600);
        let mut previous_deadline_extension = self.previous_deadline_extension.lock().unwrap();

        let now = self.clock.now();
        let elapsed = now - *previous_deadline_extension;

        // Extend the deadline when we get to 90% of the increment, to reduce
        // the risk of a task being redelivered by the queue if a call to
        // extend_task_deadline happens to take unusually long.
        if elapsed < Duration::from_secs_f64(0.9 * deadline_increment.as_secs_f64()) {
            return;
        }

        // There's not much we can do if the deadline extension fails so just
        // log the error.
        if let Err(error) = self
            .task_queue
            .lock()
            .unwrap()
            .extend_task_deadline(&self.task_handle, &deadline_increment)
        {
            error!("{}", error);
            return;
        }

        *previous_deadline_extension = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[derive(Clone, Debug)]
    struct MockTaskQueue<T: Task> {
        extended_task: Option<TaskHandle<T>>,
    }

    impl<T: Task> TaskQueue<T> for MockTaskQueue<T> {
        fn dequeue(&mut self) -> Result<Option<TaskHandle<T>>> {
            unimplemented!()
        }

        fn acknowledge_task(&mut self, _handle: TaskHandle<T>) -> Result<()> {
            unimplemented!()
        }

        fn nacknowledge_task(&mut self, _handle: TaskHandle<T>) -> Result<()> {
            unimplemented!()
        }

        /// Signal to the task queue that more time is needed to handle the task.
        fn extend_task_deadline(
            &mut self,
            handle: &TaskHandle<T>,
            _increment: &Duration,
        ) -> Result<()> {
            self.extended_task = Some(handle.clone());
            Ok(())
        }
    }

    #[derive(Clone, Debug)]
    struct MockClock {
        now: Instant,
    }

    impl Clock for MockClock {
        fn now(&self) -> Instant {
            self.now.clone()
        }
    }

    #[test]
    fn task_queue_watchdog_interval() {
        let task_handle = TaskHandle {
            acknowledgment_id: "fake-ack-id".to_string(),
            task: AggregationTask {
                trace_id: None,
                aggregation_id: "fake-agg-id".to_string(),
                aggregation_start: "2006/01/02/15/04".to_string(),
                aggregation_end: "2006/01/02/15/05".to_string(),
                batches: vec![],
            },
        };

        let task_queue: Arc<Mutex<MockTaskQueue<AggregationTask>>> =
            Arc::new(Mutex::new(MockTaskQueue {
                extended_task: None,
            }));

        let now = Instant::now();

        // Exactly at the time of last extension; nothing should happen
        let mut watchdog = TaskQueueWatchdog::new_with_previous_deadline_extension(
            task_handle.clone(),
            &now,
            task_queue.clone(),
            MockClock { now },
        );

        watchdog.send_heartbeat();
        assert_matches!((*task_queue.lock().unwrap()).extended_task, None);
        assert_eq!(*watchdog.previous_deadline_extension.lock().unwrap(), now);

        // Advance clock by less than deadline increment; nothing should happen
        watchdog.clock = MockClock {
            now: now + Duration::from_secs(300),
        };
        watchdog.send_heartbeat();
        assert_matches!((*task_queue.lock().unwrap()).extended_task, None);
        assert_eq!(*watchdog.previous_deadline_extension.lock().unwrap(), now);

        // Advance clock to 90% of deadline increment; watchdog should fire
        watchdog.clock = MockClock {
            now: now + Duration::from_secs(540),
        };
        watchdog.send_heartbeat();
        assert_matches!(
            &(*task_queue.lock().unwrap()).extended_task,
            Some(handle) => {
                assert_eq!(&task_handle, handle);
            }
        );
        assert_eq!(
            *watchdog.previous_deadline_extension.lock().unwrap(),
            now + Duration::from_secs(540)
        );

        // Advance watchdog clock by less than deadline increment; nothing
        // should happen
        watchdog.clock = MockClock {
            now: now + Duration::from_secs(800),
        };

        watchdog.send_heartbeat();
        assert_eq!(
            *watchdog.previous_deadline_extension.lock().unwrap(),
            now + Duration::from_secs(540)
        );

        // Advance watchdog clock past next deadline increment; watchdog should
        // fire
        watchdog.clock = MockClock {
            now: now + Duration::from_secs(1200),
        };

        watchdog.send_heartbeat();
        assert_eq!(
            *watchdog.previous_deadline_extension.lock().unwrap(),
            now + Duration::from_secs(1200)
        );
    }
}
