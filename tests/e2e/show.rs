use kube::{Api, Client};

use vault_mgmt_lib::construct_table;

use crate::{helm, prepare, setup::get_namespace};

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn show_succeeds() {
    let client = Client::try_default().await.unwrap();

    let namespace = &get_namespace();

    let suffix = rand::random::<u16>();
    let name = format!("vault-mgmt-e2e-{}", suffix);

    let version = "1.13.0";

    helm::add_repo().await.unwrap();
    helm::install_chart(namespace, &name, Some(&version))
        .await
        .unwrap();

    let pods = Api::namespaced(client.clone(), namespace);

    prepare::init_unseal_cluster(&pods, &Api::namespaced(client.clone(), namespace), &name)
        .await
        .unwrap();

    let table = construct_table(&pods).await.unwrap();

    let mut buf = Vec::new();
    table.print(&mut buf).unwrap();
    let output = std::str::from_utf8(buf.as_slice()).unwrap().to_string();

    if !output.contains(&version) {
        table.printstd();

        assert!(output.contains(&version));
    }

    helm::uninstall_chart(namespace, &name).await.unwrap();
}
