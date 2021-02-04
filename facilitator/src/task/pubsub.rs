use crate::{
    config::Identity,
    gcp_oauth::OauthTokenProvider,
    http::{send_json_request, JsonRequestParameters},
    task::{Task, TaskHandle, TaskQueue},
};
use anyhow::{anyhow, Context, Result};
use log::info;
use serde::Deserialize;
use std::{io::Cursor, marker::PhantomData, time::Duration};

const PUBSUB_API_BASE_URL: &str = "https://pubsub.googleapis.com";

/// Represents the response to a subscription.pull request. See API doc for
/// discussion of fields.
/// https://cloud.google.com/pubsub/docs/reference/rest/v1/projects.subscriptions/pull#response-body
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PullResponse {
    received_messages: Option<Vec<ReceivedMessage>>,
}

/// Represents a message received from a PubSub topic subscription. See API doc
/// for discussion of fields.
/// https://cloud.google.com/pubsub/docs/reference/rest/v1/projects.subscriptions/pull#receivedmessage
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ReceivedMessage {
    ack_id: String,
    message: GcpPubSubMessage,
}

/// The portion of a PubSubMessage that we are interested in. See API doc for
/// discussion of fields. Note that not all fields of a PubSubMessage are
/// parsed here, only the ones used by this application.
/// https://cloud.google.com/pubsub/docs/reference/rest/v1/PubsubMessage
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GcpPubSubMessage {
    data: String,
    message_id: String,
    publish_time: String,
}

/// A task queue backed by Google Cloud PubSub
#[derive(Debug)]
pub struct GcpPubSubTaskQueue<T: Task> {
    pubsub_api_endpoint: String,
    gcp_project_id: String,
    subscription_id: String,
    oauth_token_provider: OauthTokenProvider,
    phantom_task: PhantomData<*const T>,
}

impl<T: Task> GcpPubSubTaskQueue<T> {
    pub fn new(
        pubsub_api_endpoint: Option<&str>,
        gcp_project_id: &str,
        subscription_id: &str,
        identity: Identity,
    ) -> Result<GcpPubSubTaskQueue<T>> {
        Ok(GcpPubSubTaskQueue {
            pubsub_api_endpoint: pubsub_api_endpoint
                .unwrap_or(PUBSUB_API_BASE_URL)
                .to_owned(),
            gcp_project_id: gcp_project_id.to_string(),
            subscription_id: subscription_id.to_string(),
            oauth_token_provider: OauthTokenProvider::new(
                // This token is used to access PubSub API
                // https://developers.google.com/identity/protocols/oauth2/scopes
                "https://www.googleapis.com/auth/pubsub",
                identity.map(|x| x.to_string()),
                None, // GCP key file; never used
            )?,
            phantom_task: PhantomData,
        })
    }
}

impl<T: Task> TaskQueue<T> for GcpPubSubTaskQueue<T> {
    fn dequeue(&mut self) -> Result<Option<TaskHandle<T>>> {
        info!(
            "pull task from {}/{} as {:?}",
            self.gcp_project_id, self.subscription_id, self.oauth_token_provider
        );

        // API reference: https://cloud.google.com/pubsub/docs/reference/rest/v1/projects.subscriptions/pull
        let url = format!(
            "{}/v1/projects/{}/subscriptions/{}:pull",
            self.pubsub_api_endpoint, self.gcp_project_id, self.subscription_id
        );

        let http_response = send_json_request(JsonRequestParameters {
            request: ureq::post(&url),
            token_provider: Some(&mut self.oauth_token_provider),
            // Empirically, if there are no messages available in the
            // subscription, the PubSub API will wait about 90 seconds to send
            // an HTTP 200 with an empty JSON body. We set higher timeouts than
            // usual to allow for this.
            connect_timeout_millis: Some(180_000), // three minutes
            read_timeout_millis: Some(180_000),    // three minutes
            body: ureq::json!({
                // Dequeue one task at a time
                "maxMessages": 1
            }),
        })?;
        if http_response.error() {
            return Err(anyhow!(
                "failed to pull messages from PubSub topic: {:?}",
                http_response
            ));
        }

        let response = http_response
            .into_json_deserialize::<PullResponse>()
            .context("failed to deserialize response from PubSub API")?;

        let received_messages = match response.received_messages {
            Some(ref messages) => messages,
            None => return Ok(None),
        };

        if received_messages.len() > 1 {
            return Err(anyhow!(
                "unexpected number of messages in PubSub API response: {:?}",
                response
            ));
        }

        if received_messages.len() == 0 {
            return Ok(None);
        }

        // The JSON task is encoded as Base64 in the pubsub message
        let task_json = base64::decode(&received_messages[0].message.data)
            .context("failed to decode PubSub message")?;

        let task: T = serde_json::from_reader(Cursor::new(&task_json))
            .context(format!("failed to decode task {:?} from JSON", task_json))?;

        let handle = TaskHandle {
            task: task,
            acknowledgment_id: received_messages[0].ack_id.clone(),
        };

        Ok(Some(handle))
    }

    fn acknowledge_task(&mut self, handle: TaskHandle<T>) -> Result<()> {
        info!(
            "acknowledging task {} in topic {}/{} as {:?}",
            handle.acknowledgment_id,
            self.gcp_project_id,
            self.subscription_id,
            self.oauth_token_provider
        );

        // API reference: https://cloud.google.com/pubsub/docs/reference/rest/v1/projects.subscriptions/acknowledge
        let url = format!(
            "{}/v1/projects/{}/subscriptions/{}:acknowledge",
            self.pubsub_api_endpoint, self.gcp_project_id, self.subscription_id
        );

        let http_response = send_json_request(JsonRequestParameters {
            request: ureq::post(&url),
            token_provider: Some(&mut self.oauth_token_provider),
            body: ureq::json!({
                "ackIds": [handle.acknowledgment_id]
            }),
            ..Default::default()
        })?;
        if http_response.error() {
            return Err(anyhow!(
                "failed to acknowledge task {:?}: {:?}",
                handle,
                http_response
            ));
        }

        Ok(())
    }

    fn nacknowledge_task(&mut self, handle: TaskHandle<T>) -> Result<()> {
        info!(
            "nacknowledging task {} in topic {}/{} as {:?}",
            handle.acknowledgment_id,
            self.gcp_project_id,
            self.subscription_id,
            self.oauth_token_provider,
        );

        Ok(self
            .modify_ack_deadline(&handle, &Duration::from_secs(0))
            .context("failed to nacknowledge task")?)
    }

    fn extend_task_deadline(&mut self, handle: &TaskHandle<T>, increment: &Duration) -> Result<()> {
        info!(
            "extending deadline on task {} in topic {}/{} as {:?}",
            handle.acknowledgment_id,
            self.gcp_project_id,
            self.subscription_id,
            self.oauth_token_provider,
        );

        Ok(self
            .modify_ack_deadline(handle, increment)
            .context("failed to extend deadline on task")?)
    }
}

impl<T: Task> GcpPubSubTaskQueue<T> {
    /// Changes the ack deadline on the message described by the task handle,
    /// resetting it to the provided duration.
    fn modify_ack_deadline(
        &mut self,
        handle: &TaskHandle<T>,
        ack_deadline: &Duration,
    ) -> Result<()> {
        // API reference: https://cloud.google.com/pubsub/docs/reference/rest/v1/projects.subscriptions/modifyAckDeadline
        // Per API doc, deadline must be between 0 and 600 seconds.
        // Duration::as_secs returns u64, which cannot be negative, so we only
        // check the upper bound.
        if ack_deadline.as_secs() > 600 {
            return Err(anyhow!("invalid ack deadline {:?}", ack_deadline));
        }

        let url = format!(
            "{}/v1/projects/{}/subscriptions/{}:modifyAckDeadline",
            self.pubsub_api_endpoint, self.gcp_project_id, self.subscription_id
        );

        let http_response = send_json_request(JsonRequestParameters {
            request: ureq::post(&url),
            token_provider: Some(&mut self.oauth_token_provider),
            body: ureq::json!({
                "ackIds": [handle.acknowledgment_id],
                "ackDeadlineSeconds": ack_deadline.as_secs(),
            }),
            ..Default::default()
        })?;
        if http_response.error() {
            return Err(anyhow!(
                "failed to modify ack deadline on task {:?}: {:?}",
                handle,
                http_response
            ));
        }

        Ok(())
    }
}
