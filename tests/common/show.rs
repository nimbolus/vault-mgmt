use vault_mgmt_lib::{Flavor, construct_table};

use super::setup::{setup, teardown};

pub async fn show_succeeds(version_current: &str, flavor: Flavor) {
    let (namespace, name, pods, stss, _, _) = setup(flavor, "show", version_current).await;

    let sts = stss.get(&name).await.unwrap();
    let table = construct_table(&pods, &sts, flavor).await.unwrap();

    let mut buf = Vec::new();
    table.print(&mut buf).unwrap();
    let output = std::str::from_utf8(buf.as_slice()).unwrap().to_string();

    if !output.contains(version_current) {
        table.printstd();

        assert!(output.contains(version_current));
    }

    teardown(&namespace, &name).await;
}
