use vault_mgmt_lib::construct_table;

use crate::setup::{setup, teardown, VAULT_VERSION_CURRENT};

#[ignore = "needs a running kubernetes cluster and the helm cli"]
#[tokio::test]
async fn show_succeeds() {
    let (namespace, name, pods, _, _, _) = setup("show", VAULT_VERSION_CURRENT).await;

    let table = construct_table(&pods).await.unwrap();

    let mut buf = Vec::new();
    table.print(&mut buf).unwrap();
    let output = std::str::from_utf8(buf.as_slice()).unwrap().to_string();

    if !output.contains(VAULT_VERSION_CURRENT) {
        table.printstd();

        assert!(output.contains(VAULT_VERSION_CURRENT));
    }

    teardown(&namespace, &name).await;
}
