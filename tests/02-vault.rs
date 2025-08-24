use vault_mgmt_lib::Flavor;

pub mod common;

pub(crate) const VERSION_OLD: &str = "1.16.0";
pub(crate) const VERSION_CURRENT: &str = "1.17.0";
pub(crate) const IMAGE_NAME: &str = "hashicorp/vault";

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn vault_show_succeeds() {
    common::show_succeeds(VERSION_CURRENT, Flavor::Vault).await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn vault_upgrade_pod_succeeds_if_already_current() {
    common::upgrade_pod_succeeds_if_already_current(VERSION_CURRENT, Flavor::Vault).await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn vault_upgrade_pod_succeeds_if_already_current_with_force_upgrade() {
    common::upgrade_pod_succeeds_if_already_current_with_force_upgrade(
        VERSION_CURRENT,
        Flavor::Vault,
    )
    .await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn vault_upgrade_pod_succeeds_if_outdated_and_standby() {
    common::upgrade_pod_succeeds_if_outdated_and_standby(
        VERSION_OLD,
        VERSION_CURRENT,
        IMAGE_NAME,
        Flavor::Vault,
    )
    .await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn vault_upgrade_pod_succeeds_if_outdated_and_active() {
    common::upgrade_pod_succeeds_if_outdated_and_active(
        VERSION_OLD,
        VERSION_CURRENT,
        IMAGE_NAME,
        Flavor::Vault,
    )
    .await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn vault_upgrade_pod_succeeds_fails_with_missing_external_unseal() {
    common::upgrade_pod_succeeds_fails_with_missing_external_unseal(
        VERSION_OLD,
        VERSION_CURRENT,
        IMAGE_NAME,
        Flavor::Vault,
    )
    .await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn vault_upgrade_pod_succeeds_with_external_unseal() {
    common::upgrade_pod_succeeds_with_external_unseal(
        VERSION_OLD,
        VERSION_CURRENT,
        IMAGE_NAME,
        Flavor::Vault,
    )
    .await;
}
