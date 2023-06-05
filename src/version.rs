use std::str::FromStr;

use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct VaultVersion {
    pub version: String,
}

impl FromStr for VaultVersion {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            version: s.to_string(),
        })
    }
}

/// Construct VaultVersion from statefulset
impl TryFrom<&StatefulSet> for VaultVersion {
    type Error = anyhow::Error;

    fn try_from(statefulset: &StatefulSet) -> Result<Self, Self::Error> {
        let container = statefulset
            .spec
            .clone()
            .ok_or(anyhow::anyhow!("statefulset does not have a spec"))?
            .template
            .spec
            .ok_or(anyhow::anyhow!("statefulset does not have a pod spec"))?
            .containers;
        let container = container
            .first()
            .ok_or(anyhow::anyhow!("statefulset does not have a container"))?;

        let image = container
            .image
            .clone()
            .ok_or(anyhow::anyhow!("container does not have an image"))?;

        let version = image
            .split(':')
            .nth(1)
            .ok_or(anyhow::anyhow!("image does not have a tag"))?
            .to_string();

        Ok(Self { version })
    }
}

/// Construct VaultVersion from pod spec
impl TryFrom<&Pod> for VaultVersion {
    type Error = anyhow::Error;

    fn try_from(pod: &Pod) -> Result<Self, Self::Error> {
        let container = pod
            .spec
            .clone()
            .ok_or(anyhow::anyhow!("pod does not have a spec"))?
            .containers;
        let container = container
            .first()
            .ok_or(anyhow::anyhow!("pod does not have a container"))?;

        let image = container
            .image
            .clone()
            .ok_or(anyhow::anyhow!("container does not have an image"))?;

        let version = image
            .split(':')
            .nth(1)
            .ok_or(anyhow::anyhow!("image does not have a tag"))?
            .to_string();

        Ok(Self { version })
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};

    use crate::VaultVersion;

    #[tokio::test]
    async fn constructing_vault_version_from_statefulset_works() {
        let file = tokio::fs::read_to_string(format!(
            "tests/resources/installed/{}.yaml",
            "apis/apps/v1/namespaces/vault-mgmt-e2e/statefulsets/vault-mgmt-e2e-2274"
        ))
        .await
        .unwrap();

        let statefulset: &StatefulSet = &serde_yaml::from_str(&file).unwrap();

        let version: VaultVersion = statefulset.try_into().unwrap();

        assert_eq!(version.version, "1.13.0");
    }

    #[tokio::test]
    async fn constructing_vault_version_from_pod_works() {
        let file = tokio::fs::read_to_string(format!(
            "tests/resources/installed/{}.yaml",
            "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-0"
        ))
        .await
        .unwrap();

        let pod: &k8s_openapi::api::core::v1::Pod = &serde_yaml::from_str(&file).unwrap();

        let version: VaultVersion = pod.try_into().unwrap();

        assert_eq!(version.version, "1.13.0");
    }

    #[tokio::test]
    async fn constructing_vault_version_from_pod_with_invalid_image_fails() {
        let file = tokio::fs::read_to_string(format!(
            "tests/resources/installed/{}.yaml",
            "api/v1/namespaces/vault-mgmt-e2e/pods/vault-mgmt-e2e-2274-0"
        ))
        .await
        .unwrap();

        let mut pod: Pod = serde_yaml::from_str(&file).unwrap();

        pod.spec
            .as_mut()
            .unwrap()
            .containers
            .first_mut()
            .unwrap()
            .image = Some("vault".to_string());

        let version: Result<VaultVersion, _> = (&pod).try_into();

        assert!(version.is_err());
    }

    #[test]
    fn comparing_vault_versions_works() {
        let current = VaultVersion {
            version: "1.13.0".to_string(),
        };

        let outdated = VaultVersion {
            version: "1.12.0".to_string(),
        };

        let newer = VaultVersion {
            version: "1.14.0".to_string(),
        };

        assert!(current == current);
        assert!(current != outdated);
        assert!(current != newer);
        assert!(outdated != newer);
    }
}
