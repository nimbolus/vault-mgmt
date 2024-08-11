use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client};
use tokio::{process::Command, sync::OnceCell};

pub fn get_namespace() -> String {
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

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn kube_connection_succeeds() {
    let client = Client::try_default().await.unwrap();
    let pods: Api<Pod> = Api::namespaced(client, &get_namespace());

    pods.list(&Default::default()).await.unwrap();
}

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn helm_cli_available() {
    let helm = which::which("helm").unwrap();

    let output = Command::new(helm).arg("version").output().await.unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("version.BuildInfo"));
}
