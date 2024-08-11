use std::str::FromStr;

use kube::{
    api::{entry::Entry, PostParams},
    runtime::conditions::is_deleted,
    ResourceExt,
};

use vault_mgmt_lib::{is_pod_sealed, Unseal, VaultVersion, VAULT_PORT};

use crate::setup::{setup, teardown, VAULT_IMAGE_NAME, VAULT_VERSION_CURRENT, VAULT_VERSION_OLD};

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_already_current() {
    let (namespace, name, _, stss, init, pods) = setup("upgrade-noop", VAULT_VERSION_CURRENT).await;

    let sts = stss.get(&name).await.unwrap();
    let pod = pods.api.get(&format!("{}-0", name)).await.unwrap();

    pods.upgrade(
        pod,
        &VaultVersion::try_from(&sts).unwrap(),
        init.root_token,
        true,
        false,
        &init.keys,
    )
    .await
    .unwrap();

    teardown(&namespace, &name).await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_already_current_with_force_upgrade() {
    let (namespace, name, _, stss, init, pods) =
        setup("upgrade-force", VAULT_VERSION_CURRENT).await;

    let sts = stss.get(&name).await.unwrap();
    let pod = pods.api.get(&format!("{}-1", name)).await.unwrap();

    pods.upgrade(
        pod,
        &VaultVersion::try_from(&sts).unwrap(),
        init.root_token,
        true,
        true,
        &init.keys,
    )
    .await
    .unwrap();

    teardown(&namespace, &name).await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_outdated_and_standby() {
    let (namespace, name, _, stss, init, pods) = setup("upgrade-outdated", VAULT_VERSION_OLD).await;

    match stss.entry(&name).await.unwrap() {
        Entry::Occupied(sts) => {
            sts.and_modify(|sts| {
                for container in sts
                    .spec
                    .as_mut()
                    .unwrap()
                    .template
                    .spec
                    .as_mut()
                    .unwrap()
                    .containers
                    .iter_mut()
                {
                    if container.name == "vault" {
                        container.image = Some(
                            format!("{}:{}", VAULT_IMAGE_NAME, VAULT_VERSION_CURRENT).to_string(),
                        );
                    }
                }
            })
            .commit(&PostParams::default())
            .await
            .unwrap();
        }
        _ => panic!("statefulset not found"),
    }

    let sts = stss.get(&name).await.unwrap();
    let pod = pods.api.get(&format!("{}-1", name)).await.unwrap();

    assert_eq!(
        VaultVersion::try_from(&pod).unwrap(),
        VaultVersion::from_str(VAULT_VERSION_OLD).unwrap()
    );

    pods.upgrade(
        pod,
        &VaultVersion::try_from(&sts).unwrap(),
        init.root_token,
        true,
        false,
        &init.keys,
    )
    .await
    .unwrap();

    let pod = pods.api.get(&format!("{}-1", name)).await.unwrap();
    assert_eq!(
        VaultVersion::try_from(&pod).unwrap(),
        VaultVersion::from_str(VAULT_VERSION_CURRENT).unwrap()
    );

    teardown(&namespace, &name).await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_outdated_and_active() {
    let (namespace, name, _, stss, init, pods) = setup("upgrade-outdated", VAULT_VERSION_OLD).await;

    match stss.entry(&name).await.unwrap() {
        Entry::Occupied(sts) => {
            sts.and_modify(|sts| {
                for container in sts
                    .spec
                    .as_mut()
                    .unwrap()
                    .template
                    .spec
                    .as_mut()
                    .unwrap()
                    .containers
                    .iter_mut()
                {
                    if container.name == "vault" {
                        container.image = Some(
                            format!("{}:{}", VAULT_IMAGE_NAME, VAULT_VERSION_CURRENT).to_string(),
                        );
                    }
                }
            })
            .commit(&PostParams::default())
            .await
            .unwrap();
        }
        _ => panic!("statefulset not found"),
    }

    let sts = stss.get(&name).await.unwrap();
    let pod = pods.api.get(&format!("{}-0", name)).await.unwrap();

    assert_eq!(
        VaultVersion::try_from(&pod).unwrap(),
        VaultVersion::from_str(VAULT_VERSION_OLD).unwrap()
    );

    pods.upgrade(
        pod,
        &VaultVersion::try_from(&sts).unwrap(),
        init.root_token,
        true,
        false,
        &init.keys,
    )
    .await
    .unwrap();

    let pod = pods.api.get(&format!("{}-0", name)).await.unwrap();
    assert_eq!(
        VaultVersion::try_from(&pod).unwrap(),
        VaultVersion::from_str(VAULT_VERSION_CURRENT).unwrap()
    );

    teardown(&namespace, &name).await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_fails_with_missing_external_unseal() {
    let (namespace, name, _, stss, init, pods) =
        setup("upgrade-miss-ext-unseal", VAULT_VERSION_OLD).await;

    match stss.entry(&name).await.unwrap() {
        Entry::Occupied(sts) => {
            sts.and_modify(|sts| {
                for container in sts
                    .spec
                    .as_mut()
                    .unwrap()
                    .template
                    .spec
                    .as_mut()
                    .unwrap()
                    .containers
                    .iter_mut()
                {
                    if container.name == "vault" {
                        container.image = Some(
                            format!("{}:{}", VAULT_IMAGE_NAME, VAULT_VERSION_CURRENT).to_string(),
                        );
                    }
                }
            })
            .commit(&PostParams::default())
            .await
            .unwrap();
        }
        _ => panic!("statefulset not found"),
    }

    let sts = stss.get(&name).await.unwrap();
    let pod = pods.api.get(&format!("{}-1", name)).await.unwrap();

    assert_eq!(
        VaultVersion::try_from(&pod).unwrap(),
        VaultVersion::from_str(VAULT_VERSION_OLD).unwrap()
    );

    tokio::time::timeout(
        tokio::time::Duration::from_secs(30),
        pods.upgrade(
            pod,
            &VaultVersion::try_from(&sts).unwrap(),
            init.root_token,
            false,
            false,
            &init.keys,
        ),
    )
    .await
    .expect_err("upgrade should timeout");

    teardown(&namespace, &name).await;
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_with_external_unseal() {
    let (namespace, name, _, stss, init, pods) =
        setup("upgrade-with-ext-unseal", VAULT_VERSION_OLD).await;

    match stss.entry(&name).await.unwrap() {
        Entry::Occupied(sts) => {
            sts.and_modify(|sts| {
                for container in sts
                    .spec
                    .as_mut()
                    .unwrap()
                    .template
                    .spec
                    .as_mut()
                    .unwrap()
                    .containers
                    .iter_mut()
                {
                    if container.name == "vault" {
                        container.image = Some(
                            format!("{}:{}", VAULT_IMAGE_NAME, VAULT_VERSION_CURRENT).to_string(),
                        );
                    }
                }
            })
            .commit(&PostParams::default())
            .await
            .unwrap();
        }
        _ => panic!("statefulset not found"),
    }

    let sts = stss.get(&name).await.unwrap();
    let pod = pods.api.get(&format!("{}-1", name)).await.unwrap();

    assert_eq!(
        VaultVersion::try_from(&pod).unwrap(),
        VaultVersion::from_str(VAULT_VERSION_OLD).unwrap()
    );

    let pods_unseal = pods.clone();
    let name_unseal = name.clone();
    let uid = pod.uid().unwrap();
    let init_unseal = init.clone();

    tokio::spawn(async move {
        kube::runtime::wait::await_condition(
            pods_unseal.api.clone(),
            &format!("{}-1", &name_unseal),
            is_deleted(&uid),
        )
        .await
        .unwrap();

        kube::runtime::wait::await_condition(
            pods_unseal.api.clone(),
            &format!("{}-1", &name_unseal),
            is_pod_sealed(),
        )
        .await
        .unwrap();

        pods_unseal
            .http(&format!("{}-1", &name_unseal), VAULT_PORT)
            .await
            .unwrap()
            .unseal(&init_unseal.keys)
            .await
            .unwrap();
    });

    pods.upgrade(
        pod,
        &VaultVersion::try_from(&sts).unwrap(),
        init.root_token,
        false,
        false,
        &[],
    )
    .await
    .unwrap();

    teardown(&namespace, &name).await;
}
