mod pubsub;
mod sqs;

use anyhow::Result;
use dyn_clone::{clone_trait_object, DynClone};
use serde::Deserialize;
use slog::{Key, Record, Serializer, Value};
use std::{
    fmt,
    fmt::{Debug, Display},
    time::Duration,
};
use uuid::Uuid;

pub use pubsub::GcpPubSubTaskQueue;
pub use sqs::AwsSqsTaskQueue;

/// A queue of tasks to be executed
pub trait TaskQueue<T: Task>: Debug + DynClone + Send + Sync {
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

    /// Extend the deadline for the provided task if enough time has elapsed
    /// since the start of handling the task to require an extension. Returns
    /// Ok(()) if either the task deadline does not need extension or if the
    /// deadline was successfully extended, or an error if something goes wrong
    /// extending the deadline.
    fn maybe_extend_task_deadline(
        &mut self,
        handle: &TaskHandle<T>,
        elapsed: &Duration,
    ) -> Result<()> {
        // We assume that 10 minutes is a reasonable deadline increment
        // regardless of queue implementation or what the task is. In the future
        // this could be a tunable parameter on facilitator.
        let deadline_increment = Duration::from_secs(600);

        // Extend the deadline when we get to 90% of the increment, to reduce
        // the risk of a task being redelivered by the queue if a call to
        // extend_task_deadline happens to take unusually long.
        if elapsed >= &Duration::from_secs_f64(0.9 * Duration::from_secs(600).as_secs_f64()) {
            return self.extend_task_deadline(handle, &deadline_increment);
        }
        Ok(())
    }
}

clone_trait_object!(<T: Task> TaskQueue<T>);

/// Represents a task that can be assigned to a worker
pub trait Task:
    Debug + Display + Sized + Send + Sync + serde::de::DeserializeOwned + Clone
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
            writeln!(f, "trace ID: {}", id)?;
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
            writeln!(f, "trace ID: {}", id)?;
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
#[derive(Debug, Clone)]
pub struct TaskHandle<T: Task> {
    /// The acknowledgment ID for the task
    acknowledgment_id: String,
    /// The task
    pub task: T,
}

// Implementing slog::Value allows us to put TaskHandles in structured events
// with minimal ceremony.
impl<T: Task> Value for TaskHandle<T> {
    fn serialize(&self, _: &Record<'_>, key: Key, serializer: &mut dyn Serializer) -> slog::Result {
        serializer.emit_str(key, &format!("{}", self))
    }
}

impl<T: Task> Display for TaskHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ack ID: {}\ntask: {}", self.acknowledgment_id, self.task)
    }
}
