use std::collections::HashMap;

use secrecy::SecretString;
use vault_mgmt_lib::{Flavor, exec_pod, retry_request};

use super::setup::{setup, teardown};

pub async fn exec_pod_succeeds(version_current: &str, flavor: Flavor) {
    let (namespace, name, pods, _, _, _) = setup(flavor, "exec", version_current).await;

    let pod_name = format!("{}-0", name);
    let pod = retry_request("getting pod", || pods.get(&pod_name))
        .await
        .unwrap();

    let (stdout, stderr) = exec_pod(
        &pods,
        &pod,
        "echo out; echo err >&2".to_string(),
        HashMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(stdout, "out\n");
    assert_eq!(stderr, "err\n");

    let env = HashMap::from([("VAULT_MGMT_E2E".to_string(), SecretString::from("value"))]);
    let (stdout, _) = exec_pod(&pods, &pod, "printenv VAULT_MGMT_E2E".to_string(), env)
        .await
        .unwrap();
    assert_eq!(stdout, "value\n");

    // more output than kube buffers must not block the exec
    let (stdout, _) = exec_pod(
        &pods,
        &pod,
        "head -c 100000 /dev/zero | tr '\\000' a".to_string(),
        HashMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(stdout.len(), 100000);

    teardown(&namespace, &name).await;
}
