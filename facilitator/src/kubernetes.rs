use anyhow::{anyhow, Result};

use kube::{
    api::{Api, ListParams},
    Client,
};

use k8s_openapi::{api::core::v1::Secret, Metadata};

use tokio::runtime::Runtime;

/// Definition of a namespaced Kubernetes API implementation
#[derive(Debug)]
pub struct Kubernetes {
    namespace: String,
}

/// Methods for retrieving time from a value
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Time(chrono::DateTime<chrono::Utc>);
trait WithTime {
    fn get_time(&self) -> Time;
}

/// Implementation of WithTime for Kubernetes Secret. Will panic if timestamp
/// does not exist.
impl WithTime for Secret {
    fn get_time(&self) -> Time {
        let x = self
            .metadata()
            .clone()
            .creation_timestamp
            .expect("Should have had timestamp");
        Time(x.0)
    }
}

/// Sorts a vector of WithTime values with descending values.
fn sort_by_time_descending<T: WithTime>(values: &mut Vec<T>) {
    values.sort_by(|e1, e2| {
        let t1 = e1.get_time();

        let t2 = e2.get_time();

        t2.partial_cmp(&t1)
            .expect("t1 should've been comparable to t2")
    })
}

impl Kubernetes {
    pub fn new(namespace: String) -> Self {
        Kubernetes { namespace }
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
            .map_err(|e| {
                anyhow!(
                    "listing secrets failed with {} as label_selector: {}",
                    label_selector,
                    e
                )
            })?;

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

    impl WithTime for TestStruct {
        fn get_time(&self) -> Time {
            self.creation.clone()
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
