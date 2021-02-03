use anyhow::{anyhow, Context, Result};

use kube::{
    api::{Api, ListParams, Meta},
    Client,
};

use k8s_openapi::{api::core::v1::Secret, Metadata};

use tokio::runtime::Runtime;

pub(crate) struct Kubernetes {
    dry_run: bool,
    namespace: String,
}

impl Kubernetes {
    pub(crate) fn new(dry_run: bool, namespace: String) -> Self {
        Kubernetes { dry_run, namespace }
    }

    /// Gets a vector of decending secrets from the kubernetes API. Newer secrets are first.
    pub(crate) fn get_sorted_secrets(&self, label_selector: String) -> Result<Vec<Secret>> {
        let runtime = Runtime::new().expect("failed to create runtime for kubernetes sorted_secrets");
        runtime.block_on(self.get_sorted_secrets_impl(label_selector))
    }

    async fn get_sorted_secrets_impl(&self, label_selector: String) -> Result<Vec<Secret>> {
        let client = Self::create_client().await?;

        let secrets: Api<Secret> = Api::namespaced(client, &self.namespace);

        let mut listed_secrets = secrets
            .list(&ListParams {
                label_selector: Some(label_selector.clone()),
                ..Default::default()
            })
            .await
            .map_err(|e| {
                anyhow!(
                    "listing secrets failed with {} as label_selector: {}",
                    label_selector,
                    e
                )
            })?;

        listed_secrets.items.sort_by(|k1, k2| {
            let t1 = k1
                .metadata()
                .creation_timestamp
                .as_ref()
                .expect("secret did not have a creation_timestamp");

            let t2 = k2
                .metadata()
                .creation_timestamp
                .as_ref()
                .expect("secret did not have a creation_timestamp");

            t2.partial_cmp(t1)
                .expect("t1 should've been comparable to t2")
        });

        Ok(listed_secrets.items)
    }

    async fn create_client() -> Result<Client> {
        Client::try_default()
            .await
            .map_err(|e| anyhow!("error when getting kubernetes client: {:?}", e))
    }

}
