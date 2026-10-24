use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;

use vault_mgmt_lib::{
    Flavor, PodApi, Unseal, VAULT_PORT, list_vault_pods, retry_request, wait_for,
    {GetSealStatus, is_seal_status_initialized}, {Init, InitRequest, InitResult},
    {is_pod_ready, is_statefulset_ready},
};

pub async fn init_unseal_cluster(
    pods: &Api<Pod>,
    stss: &Api<StatefulSet>,
    name: &str,
    flavor: Flavor,
) -> anyhow::Result<InitResult> {
    let sts = wait_for(stss, name, |obj: Option<&StatefulSet>| {
        if let Some(sts) = &obj
            && let Some(status) = &sts.status
        {
            return status.replicas == 3;
        }
        false
    })
    .await?
    .ok_or(anyhow::anyhow!("statefulset {} not found", name))?;

    // only the pods of this statefulset, other tests run in the same namespace
    let lp = list_vault_pods(&sts)?;
    let pod_list = retry_request("listing pods", || pods.list(&lp)).await?;

    let mut tasks = FuturesUnordered::new();

    // running and exporting its seal status via service registration labels
    let is_labelled = move |obj: Option<&Pod>| {
        let Some(pod) = obj else {
            return false;
        };

        match &pod.status {
            Some(status) => match &status.phase {
                Some(phase) if phase == "Running" => {}
                _ => return false,
            },
            _ => return false,
        }

        let Some(labels) = &pod.metadata.labels else {
            return false;
        };

        ["initialized", "sealed", "active"]
            .iter()
            .all(|label| labels.contains_key(&format!("{}-{}", flavor, label)))
    };

    for p in pod_list {
        let pods = pods.clone();
        let pod_name = p.metadata.name.unwrap();
        tasks.push(tokio::spawn(async move {
            wait_for(&pods, &pod_name, is_labelled).await
        }));
    }

    while let Some(task) = tasks.next().await {
        task.unwrap().unwrap();
    }

    let first = format!("{}-0", name);

    let mut pf = PodApi::new(pods.clone(), false, "".to_string(), flavor)
        .http(&first, VAULT_PORT)
        .await
        .unwrap();

    // initialize vault
    let init_result = pf.init(InitRequest::default()).await?;

    println!("init_result: {:#?}", init_result);

    pf.await_seal_status(is_seal_status_initialized()).await?;

    println!("seal status initialized");

    // unseal vault
    pf.unseal(&init_result.keys).await?;

    println!("first pod unsealed");

    wait_for(pods, &first, is_pod_ready()).await?;

    println!("first pod ready");

    // unseal other pods
    for pod in [1, 2] {
        let mut pf = PodApi::new(pods.clone(), false, "".to_string(), flavor)
            .http(&format!("{}-{}", name, pod), VAULT_PORT)
            .await?;

        pf.await_seal_status(is_seal_status_initialized()).await?;

        println!("pod {} seal status initialized", pod);

        pf.unseal(&init_result.keys).await?;

        println!("pod {} unsealed", pod);
    }

    wait_for(stss, name, is_statefulset_ready()).await?;

    println!("statefulset {} ready", name);

    Ok(init_result)
}
