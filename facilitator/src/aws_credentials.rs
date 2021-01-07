use anyhow::Result;
use rusoto_core::{
    credential::EnvironmentProvider,
    credential::{
        AutoRefreshingProvider, AwsCredentials, ContainerProvider, CredentialsError,
        InstanceMetadataProvider, ProfileProvider, ProvideAwsCredentials,
    },
};
use rusoto_sts::WebIdentityProvider;
use std::{boxed::Box, time::Duration};
use tokio::runtime::{Builder, Runtime};

/// Constructs a basic runtime suitable for use in our single threaded context
pub(crate) fn basic_runtime() -> Result<Runtime> {
    Ok(Builder::new().basic_scheduler().enable_all().build()?)
}

// ------------- Everything below here was copied from rusoto/credential/src/lib.rs in the rusoto repo ---------------------------
// -------------------------------------------------------------------------------------------------------------------------------

/// Wraps a `ChainProvider` in an `AutoRefreshingProvider`.
///
/// The underlying `ChainProvider` checks multiple sources for credentials, and the `AutoRefreshingProvider`
/// refreshes the credentials automatically when they expire.
///
/// # Warning
///
/// This provider allows the [`credential_process`][credential_process] option in the AWS config
/// file (`~/.aws/config`), a method of sourcing credentials from an external process. This can
/// potentially be dangerous, so proceed with caution. Other credential providers should be
/// preferred if at all possible. If using this option, you should make sure that the config file
/// is as locked down as possible using security best practices for your operating system.
///
/// [credential_process]: https://docs.aws.amazon.com/cli/latest/topic/config-vars.html#sourcing-credentials-from-external-processes
#[derive(Clone)]
pub struct DefaultCredentialsProvider(AutoRefreshingProvider<ChainProvider>);

impl DefaultCredentialsProvider {
    /// Creates a new thread-safe `DefaultCredentialsProvider`.
    pub fn new() -> Result<DefaultCredentialsProvider, CredentialsError> {
        let inner = AutoRefreshingProvider::new(ChainProvider::new())?;
        Ok(DefaultCredentialsProvider(inner))
    }
}

#[async_trait]
impl ProvideAwsCredentials for DefaultCredentialsProvider {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        self.0.credentials().await
    }
}

/// Provides AWS credentials from multiple possible sources using a priority order.
///
/// The following sources are checked in order for credentials when calling `credentials`:
///
/// 1. Environment variables: `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`
/// 2. `credential_process` command in the AWS config file, usually located at `~/.aws/config`.
/// 3. AWS credentials file. Usually located at `~/.aws/credentials`.
/// 4. IAM instance profile. Will only work if running on an EC2 instance with an instance profile/role.
///
/// If the sources are exhausted without finding credentials, an error is returned.
///
/// The provider has a default timeout of 30 seconds. While it should work well for most setups,
/// you can change the timeout using the `set_timeout` method.
///
/// # Example
///
///
/// # Warning
///
/// This provider allows the [`credential_process`][credential_process] option in the AWS config
/// file (`~/.aws/config`), a method of sourcing credentials from an external process. This can
/// potentially be dangerous, so proceed with caution. Other credential providers should be
/// preferred if at all possible. If using this option, you should make sure that the config file
/// is as locked down as possible using security best practices for your operating system.
///
/// [credential_process]: https://docs.aws.amazon.com/cli/latest/topic/config-vars.html#sourcing-credentials-from-external-processes
#[derive(Debug, Clone)]
pub struct ChainProvider {
    environment_provider: EnvironmentProvider,
    instance_metadata_provider: InstanceMetadataProvider,
    container_provider: ContainerProvider,
    profile_provider: Option<ProfileProvider>,
    webidp_provider: WebIdentityProvider,
}

impl ChainProvider {
    /// Set the timeout on the provider to the specified duration.
    #[allow(dead_code)]
    pub fn set_timeout(&mut self, duration: Duration) {
        self.instance_metadata_provider.set_timeout(duration);
        self.container_provider.set_timeout(duration);
    }
}

async fn chain_provider_credentials(
    provider: ChainProvider,
) -> Result<AwsCredentials, CredentialsError> {
    if let Ok(creds) = provider.environment_provider.credentials().await {
        return Ok(creds);
    }
    if let Some(ref profile_provider) = provider.profile_provider {
        if let Ok(creds) = profile_provider.credentials().await {
            return Ok(creds);
        }
    }
    if let Ok(creds) = provider.container_provider.credentials().await {
        return Ok(creds);
    }
    if let Ok(creds) = provider.webidp_provider.credentials().await {
        return Ok(creds);
    }
    if let Ok(creds) = provider.instance_metadata_provider.credentials().await {
        return Ok(creds);
    }
    Err(CredentialsError::new(
        "Couldn't find AWS credentials in environment, credentials file, or IAM role.",
    ))
}

use async_trait::async_trait;

#[async_trait]
impl ProvideAwsCredentials for ChainProvider {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        chain_provider_credentials(self.clone()).await
    }
}

impl ChainProvider {
    /// Create a new `ChainProvider` using a `ProfileProvider` with the default settings.
    pub fn new() -> ChainProvider {
        ChainProvider {
            environment_provider: EnvironmentProvider::default(),
            profile_provider: ProfileProvider::new().ok(),
            instance_metadata_provider: InstanceMetadataProvider::new(),
            container_provider: ContainerProvider::new(),
            webidp_provider: WebIdentityProvider::from_k8s_env(),
        }
    }

    /// Create a new `ChainProvider` using the provided `ProfileProvider`.
    #[allow(dead_code)]
    pub fn with_profile_provider(profile_provider: ProfileProvider) -> ChainProvider {
        ChainProvider {
            environment_provider: EnvironmentProvider::default(),
            profile_provider: Some(profile_provider),
            instance_metadata_provider: InstanceMetadataProvider::new(),
            container_provider: ContainerProvider::new(),
            webidp_provider: WebIdentityProvider::from_k8s_env(),
        }
    }
}

impl Default for ChainProvider {
    fn default() -> Self {
        Self::new()
    }
}
