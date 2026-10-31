use kube::ResourceExt;
use vault_mgmt_lib::{
    ExecIn, Flavor, is_pod_ready, is_pod_unsealed, list_vault_pods, reload, retry_request, wait_for,
};

use super::setup::{setup, teardown};

pub async fn reload_succeeds(version_current: &str, flavor: Flavor) {
    let (namespace, name, pods, stss, _, _) = setup(flavor, "reload", version_current).await;

    let sts = retry_request("getting statefulset", || stss.get(&name))
        .await
        .unwrap();

    reload(&pods, &sts, flavor, Some(ExecIn::Standby), false)
        .await
        .unwrap();
    reload(&pods, &sts, flavor, Some(ExecIn::Active), false)
        .await
        .unwrap();
    reload(&pods, &sts, flavor, None, false).await.unwrap();

    // all pods are unsealed
    reload(&pods, &sts, flavor, Some(ExecIn::Sealed), false)
        .await
        .unwrap_err();
    reload(&pods, &sts, flavor, Some(ExecIn::Sealed), true)
        .await
        .unwrap();

    let lp = list_vault_pods(&sts).unwrap();
    let list = retry_request("listing pods", || pods.list(&lp))
        .await
        .unwrap();
    assert!(!list.items.is_empty());

    // a reload must neither restart nor seal the pods
    for pod in list.iter() {
        let pod_name = pod.name_any();
        wait_for(&pods, &pod_name, is_pod_unsealed(flavor))
            .await
            .unwrap();
        let pod = wait_for(&pods, &pod_name, is_pod_ready())
            .await
            .unwrap()
            .unwrap();

        let restarts = pod
            .status
            .and_then(|s| s.container_statuses)
            .unwrap_or_default()
            .iter()
            .map(|c| c.restart_count)
            .sum::<i32>();
        assert_eq!(restarts, 0, "pod {} restarted", pod_name);
    }

    teardown(&namespace, &name).await;
}
