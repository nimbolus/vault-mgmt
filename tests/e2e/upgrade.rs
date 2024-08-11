use std::str::FromStr;

use kube::{
    api::{entry::Entry, PostParams},
    runtime::{conditions::is_pod_running, wait::Condition},
    Api, Client,
};
use secrecy::ExposeSecret;

use vault_mgmt_lib::{
    is_pod_sealed, PodApi, Unseal, VaultVersion, VAULT_PORT,
    {raft_configuration_all_voters, GetRaftConfiguration},
};

use crate::{
    helm, prepare,
    setup::{get_namespace, setup_crypto_provider},
};

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_already_current() {
    setup_crypto_provider().await;

    let client = Client::try_default().await.unwrap();

    let namespace = &get_namespace();

    let suffix = rand::random::<u16>();
    let name = format!("vault-mgmt-e2e-{}", suffix);
    dbg!(&name);

    let version = "1.13.0";

    helm::add_repo().await.unwrap();
    helm::install_chart(namespace, &name, Some(&version))
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), namespace);
    let stss = Api::namespaced(client.clone(), namespace);

    let init = prepare::init_unseal_cluster(&pods, &stss, &name)
        .await
        .unwrap();

    dbg!(
        &init
            .keys
            .iter()
            .map(|k| k.expose_secret())
            .collect::<Vec<_>>(),
        &init.root_token.expose_secret()
    );

    let pods = PodApi::new(pods, false, "".to_string());

    let mut pf = pods.http(&format!("{}-0", name), VAULT_PORT).await.unwrap();

    pf.await_raft_configuration(init.root_token.clone(), raft_configuration_all_voters())
        .await
        .unwrap();

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

    helm::uninstall_chart(namespace, &name).await.unwrap();
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_already_current_with_force_upgrade() {
    setup_crypto_provider().await;

    let client = Client::try_default().await.unwrap();

    let namespace = &get_namespace();

    let suffix = rand::random::<u16>();
    let name = format!("vault-mgmt-e2e-{}", suffix);
    dbg!(&name);

    let version = "1.13.0";

    helm::add_repo().await.unwrap();
    helm::install_chart(namespace, &name, Some(&version))
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), namespace);
    let stss = Api::namespaced(client.clone(), namespace);

    let init = prepare::init_unseal_cluster(&pods, &stss, &name)
        .await
        .unwrap();

    dbg!(
        &init
            .keys
            .iter()
            .map(|k| k.expose_secret())
            .collect::<Vec<_>>(),
        &init.root_token.expose_secret()
    );

    let pods = PodApi::new(pods, false, "".to_string());

    let mut pf = pods.http(&format!("{}-0", name), VAULT_PORT).await.unwrap();

    pf.await_raft_configuration(init.root_token.clone(), raft_configuration_all_voters())
        .await
        .unwrap();

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

    helm::uninstall_chart(namespace, &name).await.unwrap();
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_outdated_and_standby() {
    setup_crypto_provider().await;

    let client = Client::try_default().await.unwrap();

    let namespace = &get_namespace();

    let suffix = rand::random::<u16>();
    let name = format!("vault-mgmt-e2e-{}", suffix);
    dbg!(&name);

    let version = "1.12.0";

    helm::add_repo().await.unwrap();
    helm::install_chart(namespace, &name, Some(&version))
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), namespace);
    let stss = Api::namespaced(client.clone(), namespace);

    let init = prepare::init_unseal_cluster(&pods, &stss, &name)
        .await
        .unwrap();

    dbg!(
        &init
            .keys
            .iter()
            .map(|k| k.expose_secret())
            .collect::<Vec<_>>(),
        &init.root_token.expose_secret()
    );

    let pods = PodApi::new(pods, false, "".to_string());

    let mut pf = pods.http(&format!("{}-0", name), VAULT_PORT).await.unwrap();

    pf.await_raft_configuration(init.root_token.clone(), raft_configuration_all_voters())
        .await
        .unwrap();

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
                        container.image = Some("vault:1.13.0".to_string());
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
        VaultVersion::from_str("1.12.0").unwrap()
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
        VaultVersion::from_str("1.13.0").unwrap()
    );

    helm::uninstall_chart(namespace, &name).await.unwrap();
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_if_outdated_and_active() {
    setup_crypto_provider().await;

    let client = Client::try_default().await.unwrap();

    let namespace = &get_namespace();

    let suffix = rand::random::<u16>();
    let name = format!("vault-mgmt-e2e-{}", suffix);
    dbg!(&name);

    let version = "1.12.0";

    helm::add_repo().await.unwrap();
    helm::install_chart(namespace, &name, Some(&version))
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), namespace);
    let stss = Api::namespaced(client.clone(), namespace);

    let init = prepare::init_unseal_cluster(&pods, &stss, &name)
        .await
        .unwrap();

    dbg!(
        &init
            .keys
            .iter()
            .map(|k| k.expose_secret())
            .collect::<Vec<_>>(),
        &init.root_token.expose_secret()
    );

    let pods = PodApi::new(pods, false, "".to_string());

    let mut pf = pods.http(&format!("{}-0", name), VAULT_PORT).await.unwrap();

    pf.await_raft_configuration(init.root_token.clone(), raft_configuration_all_voters())
        .await
        .unwrap();

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
                        container.image = Some("vault:1.13.0".to_string());
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
        VaultVersion::from_str("1.12.0").unwrap()
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
        VaultVersion::from_str("1.13.0").unwrap()
    );

    helm::uninstall_chart(namespace, &name).await.unwrap();
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_fails_with_missing_external_unseal() {
    setup_crypto_provider().await;

    let client = Client::try_default().await.unwrap();

    let namespace = &get_namespace();

    let suffix = rand::random::<u16>();
    let name = format!("vault-mgmt-e2e-{}", suffix);
    dbg!(&name);

    let version = "1.12.0";

    helm::add_repo().await.unwrap();
    helm::install_chart(namespace, &name, Some(&version))
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), namespace);
    let stss = Api::namespaced(client.clone(), namespace);

    let init = prepare::init_unseal_cluster(&pods, &stss, &name)
        .await
        .unwrap();

    dbg!(
        &init
            .keys
            .iter()
            .map(|k| k.expose_secret())
            .collect::<Vec<_>>(),
        &init.root_token.expose_secret()
    );

    let pods = PodApi::new(pods, false, "".to_string());

    let mut pf = pods.http(&format!("{}-0", name), VAULT_PORT).await.unwrap();

    pf.await_raft_configuration(init.root_token.clone(), raft_configuration_all_voters())
        .await
        .unwrap();

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
                        container.image = Some("vault:1.13.0".to_string());
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
        VaultVersion::from_str("1.12.0").unwrap()
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

    helm::uninstall_chart(namespace, &name).await.unwrap();
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn upgrade_pod_succeeds_with_external_unseal() {
    setup_crypto_provider().await;

    let client = Client::try_default().await.unwrap();

    let namespace = &get_namespace();

    let suffix = rand::random::<u16>();
    let name = format!("vault-mgmt-e2e-{}", suffix);
    dbg!(&name);

    let version = "1.12.0";

    helm::add_repo().await.unwrap();
    helm::install_chart(namespace, &name, Some(&version))
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), namespace);
    let stss = Api::namespaced(client.clone(), namespace);

    let init = prepare::init_unseal_cluster(&pods, &stss, &name)
        .await
        .unwrap();

    dbg!(
        &init
            .keys
            .iter()
            .map(|k| k.expose_secret())
            .collect::<Vec<_>>(),
        &init.root_token.expose_secret()
    );

    let pods = PodApi::new(pods, false, "".to_string());

    let mut pf = pods.http(&format!("{}-0", name), VAULT_PORT).await.unwrap();

    pf.await_raft_configuration(init.root_token.clone(), raft_configuration_all_voters())
        .await
        .unwrap();

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
                        container.image = Some("vault:1.13.0".to_string());
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
        VaultVersion::from_str("1.12.0").unwrap()
    );

    let pods_unseal = pods.clone();
    let name_unseal = name.clone();
    let init_unseal = init.clone();

    tokio::spawn(async move {
        kube::runtime::wait::await_condition(
            pods_unseal.api.clone(),
            &format!("{}-1", &name_unseal),
            Condition::not(is_pod_running()),
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
        &vec![],
    )
    .await
    .unwrap();

    helm::uninstall_chart(namespace, &name).await.unwrap();
}
