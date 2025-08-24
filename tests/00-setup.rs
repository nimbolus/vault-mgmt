use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client};
use tokio::process::Command;

pub mod common;

use common::get_namespace;

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn kube_connection_succeeds() {
    common::setup_crypto_provider().await;

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
