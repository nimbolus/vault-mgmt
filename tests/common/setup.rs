use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::{Api, Client};
use secrecy::ExposeSecret;
use tokio::sync::OnceCell;
use vault_mgmt_lib::{
    raft_configuration_all_voters, Flavor, GetRaftConfiguration, InitResult, PodApi, VAULT_PORT,
};

use super::{helm, prepare};

pub(crate) fn get_namespace() -> String {
    std::env::var("VAULT_MGMT_E2E_NAMESPACE").unwrap_or_else(|_| "vault-mgmt-e2e".to_string())
}

static ONCE_CRYPTO_SETUP: OnceCell<()> = OnceCell::const_new();

pub async fn setup_crypto_provider() {
    ONCE_CRYPTO_SETUP
        .get_or_init(|| async {
            rustls::crypto::ring::default_provider()
                .install_default()
                .unwrap();
        })
        .await;
}

pub(crate) async fn setup(
    flavor: Flavor,
    prefix: &str,
    version: &str,
) -> (
    String,
    String,
    Api<Pod>,
    Api<StatefulSet>,
    InitResult,
    PodApi,
) {
    setup_crypto_provider().await;
    let namespace = get_namespace();

    let client = Client::try_default().await.unwrap();

    let suffix = rand::random::<u16>();
    let name = dbg!(format!("{}-{}", prefix, suffix));

    let values = match flavor {
        Flavor::OpenBao => include_str!("values-openbao.yaml").to_string(),
        Flavor::Vault => include_str!("values-vault.yaml").to_string(),
    };

    helm::add_repo().await.unwrap();
    helm::install_chart(&namespace, &name, Some(version), flavor, values)
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), &namespace);
    let stss = Api::namespaced(client.clone(), &namespace);

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

    let pod_api = PodApi::new(
        pods.clone(),
        false,
        "".to_string(),
        vault_mgmt_lib::Flavor::OpenBao,
    );

    let mut pf = pod_api
        .http(&format!("{}-0", name), VAULT_PORT)
        .await
        .unwrap();

    pf.await_raft_configuration(init.root_token.clone(), raft_configuration_all_voters())
        .await
        .unwrap();

    (namespace, name, pods, stss, init, pod_api)
}

pub(crate) async fn teardown(namespace: &str, name: &str) {
    helm::uninstall_chart(namespace, name).await.unwrap();
}
