use anyhow::{anyhow, Context, Result};
use derivative::Derivative;
use rusoto_core::Region;
use rusoto_sqs::{
    ChangeMessageVisibilityRequest, DeleteMessageRequest, ReceiveMessageRequest, Sqs, SqsClient,
};
use slog_scope::info;
use std::{convert::TryFrom, marker::PhantomData, str::FromStr, time::Duration};

use crate::aws_credentials;
use crate::{
    aws_credentials::{basic_runtime, retry_request},
    task::{Task, TaskHandle, TaskQueue},
};

/// A task queue backed by AWS SQS
#[derive(Clone, Derivative)]
#[derivative(Debug)]
pub struct AwsSqsTaskQueue<T: Task> {
    region: Region,
    queue_url: String,
    #[derivative(Debug = "ignore")]
    credentials_provider: aws_credentials::Provider,
    phantom_task: PhantomData<T>,
}

impl<T: Task> AwsSqsTaskQueue<T> {
    pub fn new(
        region: &str,
        queue_url: &str,
        credentials_provider: aws_credentials::Provider,
    ) -> Result<Self> {
        let region = Region::from_str(region).context("invalid AWS region")?;

        Ok(AwsSqsTaskQueue {
            region,
            queue_url: queue_url.to_owned(),
            credentials_provider,
            phantom_task: PhantomData,
        })
    }
}

impl<T: Task> TaskQueue<T> for AwsSqsTaskQueue<T> {
    fn dequeue(&mut self) -> Result<Option<TaskHandle<T>>> {
        info!(
            "pull task from {} as {}",
            self.queue_url, self.credentials_provider
        );

        let client = self.sqs_client()?;
        // We create a runtime for every request because tokio::runtime::Runtime
        // cannot be cloned. See
        // https://github.com/abetterinternet/prio-server/pull/486/files#r596429301
        // for details and other approaches we tried.
        let runtime = basic_runtime()?;

        let response = retry_request("dequeue SQS message", || {
            let request = ReceiveMessageRequest {
                // Dequeue one task at a time
                max_number_of_messages: Some(1),
                queue_url: self.queue_url.clone(),
                // Long polling. SQS allows us to wait up to 20 seconds.
                // https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-short-and-long-polling.html#sqs-long-polling
                wait_time_seconds: Some(20),
                // Visibility timeout configures how long SQS will wait for
                // message deletion by this client before making a message
                // visible again to other queue consumers. We set it to 600s =
                // 10 minutes.
                visibility_timeout: Some(600),
                ..Default::default()
            };

            runtime.block_on(client.receive_message(request))
        })
        .context("failed to dequeue message from SQS")?;

        let received_messages = match response.messages {
            Some(ref messages) => messages,
            None => return Ok(None),
        };

        if received_messages.is_empty() {
            return Ok(None);
        }

        if received_messages.len() > 1 {
            return Err(anyhow!(
                "unexpected number of messages in SQS response: {:?}",
                response
            ));
        }

        let body = match &received_messages[0].body {
            Some(body) => body,
            None => return Err(anyhow!("no body in SQS message")),
        };
        let receipt_handle = match &received_messages[0].receipt_handle {
            Some(handle) => handle,
            None => return Err(anyhow!("no receipt handle in SQS message")),
        };

        let task = serde_json::from_reader(body.as_bytes())
            .context(format!("failed to decode JSON task {:?}", body))?;

        Ok(Some(TaskHandle {
            task,
            acknowledgment_id: receipt_handle.to_owned(),
        }))
    }

    fn acknowledge_task(&mut self, task: TaskHandle<T>) -> Result<()> {
        info!(
            "acknowledging task {} in queue {} as {}",
            task.acknowledgment_id, self.queue_url, self.credentials_provider,
        );

        let client = self.sqs_client()?;
        let runtime = basic_runtime()?;

        retry_request("delete/acknowledge message in SQS", || {
            let request = DeleteMessageRequest {
                queue_url: self.queue_url.clone(),
                receipt_handle: task.acknowledgment_id.clone(),
            };
            runtime.block_on(client.delete_message(request))
        })
        .context("failed to delete/acknowledge message in SQS")
    }

    fn nacknowledge_task(&mut self, task: TaskHandle<T>) -> Result<()> {
        // In SQS, messages are nacked by changing the message visibility
        // timeout to 0
        // https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-visibility-timeout.html#terminating-message-visibility-timeout
        info!(
            "nacknowledging task {} in queue {} as {}",
            task.acknowledgment_id, self.queue_url, self.credentials_provider,
        );

        self.change_message_visibility(&task, &Duration::from_secs(0))
            .context("failed to nacknowledge task")
    }

    fn extend_task_deadline(&mut self, task: &TaskHandle<T>, increment: &Duration) -> Result<()> {
        info!(
            "extending deadline on task {} in queue {} by 10 minutes",
            task.acknowledgment_id, self.queue_url
        );

        self.change_message_visibility(task, increment)
            .context("failed to extend deadline on task")
    }
}

impl<T: Task> AwsSqsTaskQueue<T> {
    /// Returns a configured SqsClient, or an error on failure.
    fn sqs_client(&self) -> Result<SqsClient> {
        // Rusoto has outstanding issues where either the remote end or the
        // underlying connection pool can close idle connections under us,
        // causing API requests to fail if they are made at the wrong time. In
        // order to avoid having to carefully juggle idle connection timeouts,
        // we create a new SqsClient for each request.
        // https://github.com/rusoto/rusoto/issues/1686
        let http_client = rusoto_core::HttpClient::new().context("failed to create HTTP client")?;

        Ok(SqsClient::new_with(
            http_client,
            self.credentials_provider.clone(),
            self.region.clone(),
        ))
    }

    /// Changes the message visibility of the SQS message described by the TaskHandle, resetting it
    /// to the specified visibility timeout.
    fn change_message_visibility(
        &mut self,
        task: &TaskHandle<T>,
        visibility_timeout: &Duration,
    ) -> Result<()> {
        let client = self.sqs_client()?;
        let runtime = basic_runtime()?;

        let timeout = i64::try_from(visibility_timeout.as_secs()).context(format!(
            "timeout value {:?} cannot be encoded into ChangeMessageVisibilityRequest",
            visibility_timeout
        ))?;

        retry_request("changing message visibility", || {
            let request = ChangeMessageVisibilityRequest {
                queue_url: self.queue_url.clone(),
                receipt_handle: task.acknowledgment_id.clone(),
                visibility_timeout: timeout,
            };
            runtime.block_on(client.change_message_visibility(request))
        })
        .context("failed to change message visibility message in SQS")
    }
}
