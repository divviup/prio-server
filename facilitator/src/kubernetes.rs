use anyhow::{anyhow, Context, Result};

use kube::{
    api::{Api, ListParams},
    Client,
};

use k8s_openapi::api::core::v1::Secret;

use tokio::runtime::Runtime;

/// Definition of a namespaced Kubernetes API implementation
#[derive(Debug)]
pub struct KubernetesClient {
    namespace: String,
}

/// Definining a Time type that is used by secrets.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Time(chrono::DateTime<chrono::Utc>);

/// Will panic if secret does not have timestamp.
impl std::convert::From<&Secret> for Time {
    fn from(secret: &Secret) -> Self {
        Time(
            secret
                .metadata
                .clone()
                .creation_timestamp
                .expect("Should have had timestamp")
                .0,
        )
    }
}

/// Sorts a vector of WithTime values with descending values.
fn sort_by_time_descending<T>(values: &mut Vec<T>)
where
    for<'a> &'a T: Into<Time>,
{
    values.sort_by(|e1, e2| {
        let t1: Time = e1.into();
        let t2: Time = e2.into();

        t2.partial_cmp(&t1)
            .expect("t1 should've been comparable to t2")
    })
}

impl KubernetesClient {
    pub fn new(namespace: String) -> Self {
        KubernetesClient { namespace }
    }

    /// Gets a descending by creation timestamp sorted vector of secrets from
    /// the kubernetes API. Newer secrets are first.
    pub fn get_sorted_secrets(&self, label_selector: &str) -> Result<Vec<Secret>> {
        let runtime =
            Runtime::new().expect("failed to create runtime for kubernetes sorted_secrets");
        runtime.block_on(self.get_sorted_secrets_impl(label_selector))
    }

    async fn get_sorted_secrets_impl(&self, label_selector: &str) -> Result<Vec<Secret>> {
        let client = Self::create_client().await?;

        let secrets: Api<Secret> = Api::namespaced(client, &self.namespace);

        let listed_secrets = secrets
            .list(&ListParams {
                label_selector: Some(String::from(label_selector)),
                ..Default::default()
            })
            .await
            .context(format!(
                "listing secrets failed with {} as label_selector",
                label_selector
            ))?;

        let mut items = listed_secrets.items;
        sort_by_time_descending(&mut items);

        Ok(items)
    }

    async fn create_client() -> Result<Client> {
        Client::try_default()
            .await
            .map_err(|e| anyhow!("error when getting kubernetes client: {:?}", e))
    }
}

#[cfg(test)]

mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestStruct {
        name: String,
        creation: Time,
    }

    impl std::convert::From<&TestStruct> for Time {
        fn from(t: &TestStruct) -> Self {
            t.creation.clone()
        }
    }

    #[test]
    fn test_sorting() {
        let old = TestStruct {
            name: String::from("OLD"),
            creation: Time(chrono::MIN_DATETIME),
        };

        let now = TestStruct {
            name: String::from("NOW"),
            creation: Time(chrono::Utc::now()),
        };

        let new = TestStruct {
            name: String::from("NEW"),
            creation: Time(chrono::MAX_DATETIME),
        };

        let mut vals: Vec<TestStruct> = vec![old, now, new];
        sort_by_time_descending(&mut vals);

        println!("{:?}", vals);

        assert_eq!(vals[0].name, "NEW");
        assert_eq!(vals[1].name, "NOW");
        assert_eq!(vals[2].name, "OLD");
    }
}
