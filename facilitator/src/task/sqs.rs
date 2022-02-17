use crate::{
    aws_credentials::{self, retry_request},
    logging::event,
    metrics::ApiClientMetricsCollector,
    task::{Task, TaskHandle, TaskQueue},
};
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDateTime;
use rusoto_core::Region;
use rusoto_sns::{PublishInput, Sns, SnsClient};
use rusoto_sqs::{
    ChangeMessageVisibilityRequest, DeleteMessageRequest, ReceiveMessageRequest, Sqs, SqsClient,
};
use slog::{debug, info, o, Logger};
use std::{convert::TryFrom, marker::PhantomData, str::FromStr, time::Duration};
use tokio::runtime::Handle;

/// A task queue backed by AWS SQS
#[derive(Clone, Debug)]
pub struct AwsSqsTaskQueue<T: Task> {
    region: Region,
    queue_url: String,
    dead_letter_topic: Option<String>,
    runtime_handle: Handle,
    credentials_provider: aws_credentials::Provider,
    logger: Logger,
    api_metrics: ApiClientMetricsCollector,
    phantom_task: PhantomData<T>,
}

impl<T: Task> AwsSqsTaskQueue<T> {
    pub fn new(
        region: &str,
        queue_url: &str,
        dead_letter_topic: Option<&str>,
        runtime_handle: &Handle,
        credentials_provider: aws_credentials::Provider,
        parent_logger: &Logger,
        api_metrics: &ApiClientMetricsCollector,
    ) -> Result<Self> {
        let region = Region::from_str(region).context("invalid AWS region")?;
        let logger = parent_logger.new(o!(
            event::TASK_QUEUE_ID => queue_url.to_owned(),
            event::IDENTITY => credentials_provider.to_string(),
        ));

        Ok(AwsSqsTaskQueue {
            region,
            queue_url: queue_url.to_owned(),
            dead_letter_topic: dead_letter_topic.map(str::to_owned),
            runtime_handle: runtime_handle.clone(),
            credentials_provider,
            logger,
            api_metrics: api_metrics.clone(),
            phantom_task: PhantomData,
        })
    }

    fn service() -> &'static str {
        "sqs.amazonaws.com"
    }
}

impl<T: Task> TaskQueue<T> for AwsSqsTaskQueue<T> {
    fn dequeue(&self) -> Result<Option<TaskHandle<T>>> {
        debug!(self.logger, "pull task");
        let client = self.sqs_client()?;
        let response = retry_request(
            &self.logger.new(o!(event::ACTION => "dequeue message")),
            &self.api_metrics,
            Self::service(),
            "ReceiveMessage",
            &self.runtime_handle,
            || {
                client.receive_message(ReceiveMessageRequest {
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
                })
            },
        )
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

        let message = &received_messages[0];
        let body = message
            .body
            .as_ref()
            .ok_or_else(|| anyhow!("no body in SQS message"))?;
        let receipt_handle = message
            .receipt_handle
            .as_ref()
            .ok_or_else(|| anyhow!("no receipt handle in SQS message"))?;

        let mut published_time = None;
        if let Some(ref attributes) = message.attributes {
            if let Some(sent_timestamp) = attributes.get("SentTimestamp") {
                // SentTimestamp is a millisecond UNIX timestamp:
                // https://docs.aws.amazon.com/AWSSimpleQueueService/latest/APIReference/API_Message.html
                let sent_timestamp: i64 = sent_timestamp.parse()?;
                published_time = NaiveDateTime::from_timestamp_opt(
                    /* secs */ sent_timestamp / 1000,
                    /* nsecs */ (1_000_000 * (sent_timestamp % 1000)) as u32,
                )
            }
        };

        let task = serde_json::from_reader(body.as_bytes())
            .context(format!("failed to decode JSON task {:?}", body))?;

        Ok(Some(TaskHandle {
            task,
            raw_body: body.clone(),
            published_time,
            acknowledgment_id: receipt_handle.to_owned(),
        }))
    }

    fn acknowledge_task(&self, task: TaskHandle<T>) -> Result<()> {
        info!(
            self.logger, "acknowledging task";
            event::TASK_ACKNOWLEDGEMENT_ID => &task.acknowledgment_id,
        );

        let client = self.sqs_client()?;

        retry_request(
            &self
                .logger
                .new(o!(event::ACTION => "delete/acknowledge message")),
            &self.api_metrics,
            Self::service(),
            "DeleteMessage",
            &self.runtime_handle,
            || {
                client.delete_message(DeleteMessageRequest {
                    queue_url: self.queue_url.clone(),
                    receipt_handle: task.acknowledgment_id.clone(),
                })
            },
        )
        .context("failed to delete/acknowledge message in SQS")
    }

    fn nacknowledge_task(&self, task: TaskHandle<T>) -> Result<()> {
        // In SQS, messages are nacked by changing the message visibility
        // timeout to 0
        // https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-visibility-timeout.html#terminating-message-visibility-timeout
        info!(
            self.logger, "nacknowledging task";
            event::TASK_ACKNOWLEDGEMENT_ID => &task.acknowledgment_id,
        );

        self.change_message_visibility(&task, &Duration::from_secs(0))
            .context("failed to nacknowledge task")
    }

    fn extend_task_deadline(&self, task: &TaskHandle<T>, increment: &Duration) -> Result<()> {
        debug!(
            self.logger, "extending deadline on task by 10 minutes";
            event::TASK_ACKNOWLEDGEMENT_ID => &task.acknowledgment_id,
        );

        self.change_message_visibility(task, increment)
            .context("failed to extend deadline on task")
    }

    fn forward_to_dead_letter_queue(&self, task: TaskHandle<T>) -> Result<()> {
        if let Some(topic) = &self.dead_letter_topic {
            info!(
                self.logger, "forwarding to dead letter queue";
                event::TASK_ACKNOWLEDGEMENT_ID => &task.acknowledgment_id,
            );

            let client = self.sns_client()?;
            retry_request(
                &self
                    .logger
                    .new(o!(event::ACTION => "submit forwarded message")),
                &self.api_metrics,
                "sns.amazonaws.com",
                "Publish",
                &self.runtime_handle,
                || {
                    client.publish(PublishInput {
                        message: task.raw_body.clone(),
                        message_attributes: None,
                        message_deduplication_id: None,
                        message_group_id: None,
                        message_structure: None,
                        phone_number: None,
                        subject: None,
                        target_arn: None,
                        topic_arn: Some(topic.clone()),
                    })
                },
            )
            .context("failed to forward message to SNS")?;

            self.acknowledge_task(task)
        } else {
            info!(
                self.logger, "not forwarding to dead letter queue, topic not configured";
                event::TASK_ACKNOWLEDGEMENT_ID => &task.acknowledgment_id,
            );
            self.nacknowledge_task(task)
        }
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

    /// Returns a configured SnsClient, or an error on failure.
    fn sns_client(&self) -> Result<SnsClient> {
        let http_client = rusoto_core::HttpClient::new().context("failed to create HTTP client")?;

        Ok(SnsClient::new_with(
            http_client,
            self.credentials_provider.clone(),
            self.region.clone(),
        ))
    }

    /// Changes the message visibility of the SQS message described by the TaskHandle, resetting it
    /// to the specified visibility timeout.
    fn change_message_visibility(
        &self,
        task: &TaskHandle<T>,
        visibility_timeout: &Duration,
    ) -> Result<()> {
        let client = self.sqs_client()?;

        let timeout = i64::try_from(visibility_timeout.as_secs()).context(format!(
            "timeout value {:?} cannot be encoded into ChangeMessageVisibilityRequest",
            visibility_timeout
        ))?;

        retry_request(
            &self
                .logger
                .new(o!(event::ACTION => "changing message visibility")),
            &self.api_metrics,
            Self::service(),
            "ChangeMessageVisibility",
            &self.runtime_handle,
            || {
                client.change_message_visibility(ChangeMessageVisibilityRequest {
                    queue_url: self.queue_url.clone(),
                    receipt_handle: task.acknowledgment_id.clone(),
                    visibility_timeout: timeout,
                })
            },
        )
        .context("failed to change message visibility message in SQS")
    }
}
