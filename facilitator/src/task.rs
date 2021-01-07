mod pubsub;
mod sqs;

use anyhow::Result;
use serde::Deserialize;
use std::{
    fmt,
    fmt::{Debug, Display},
};

pub use pubsub::GcpPubSubTaskQueue;
pub use sqs::AwsSqsTaskQueue;

/// A queue of tasks to be executed
pub trait TaskQueue<T: Task>: Debug {
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
}

/// Represents a task that can be assigned to a worker
pub trait Task: Debug + Display + Sized + serde::de::DeserializeOwned {}

/// Represents an intake batch task to be executed
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct IntakeBatchTask {
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
        write!(
            f,
            "aggregation ID: {}\nbatch ID: {}\ndate: {}",
            self.aggregation_id, self.batch_id, self.date
        )
    }
}

/// Represents an aggregation task to be executed
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct AggregationTask {
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
#[derive(Debug, Deserialize, PartialEq)]
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
#[derive(Debug)]
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
