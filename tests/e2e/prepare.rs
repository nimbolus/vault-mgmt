use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::ListParams, Api};

use vault_mgmt::{
    PodApi, Unseal, VAULT_PORT, {is_pod_ready, is_statefulset_ready},
    {is_seal_status_initialized, GetSealStatus}, {Init, InitRequest, InitResult},
};

pub async fn init_unseal_cluster(
    pods: &Api<Pod>,
    stss: &Api<StatefulSet>,
    name: &str,
) -> anyhow::Result<InitResult> {
    kube::runtime::wait::await_condition(stss.clone(), &name, |obj: Option<&StatefulSet>| {
        if let Some(sts) = &obj {
            if let Some(status) = &sts.status {
                return status.replicas == 3;
            }
        }
        false
    })
    .await?;

    let pod_list = pods
        .list(
            &ListParams::default()
                .labels(&format!("app.kubernetes.io/instance={}", &name))
                .labels("component=server"),
        )
        .await?;

    let mut tasks = FuturesUnordered::new();

    fn is_labelled(obj: Option<&Pod>) -> bool {
        if let Some(pod) = obj {
            match &pod.status {
                Some(status) => match &status.phase {
                    Some(phase) if phase == "Running" => {}
                    _ => return false,
                },
                _ => return false,
            }

            fn has_label(pod: &Pod, label: &str, value: Option<&str>) -> bool {
                if let Some(labels) = &pod.metadata.labels {
                    if let Some(v) = labels.get(label) {
                        if let Some(value) = value {
                            return v == value;
                        }

                        return true;
                    }
                }
                false
            }

            if !has_label(pod, "vault-initialized", None)
                || !has_label(pod, "vault-sealed", None)
                || !has_label(pod, "vault-active", None)
            {
                return false;
            }

            return true;
        }
        false
    }

    for p in pod_list {
        let pods = pods.clone();
        let pod_name = p.metadata.name.unwrap();
        tasks.push(tokio::spawn(async move {
            kube::runtime::wait::await_condition(pods, &pod_name, is_labelled).await
        }));
    }

    while let Some(task) = tasks.next().await {
        task.unwrap().unwrap();
    }

    let first = format!("{}-0", &name);

    let mut pf = PodApi::new(pods.clone(), false, "".to_string())
        .http(&first, VAULT_PORT)
        .await
        .unwrap();

    // initialize vault
    let init_result = pf.init(InitRequest::default()).await?;

    pf.await_seal_status(is_seal_status_initialized()).await?;

    // unseal vault
    pf.unseal(&init_result.keys).await?;

    kube::runtime::wait::await_condition(pods.clone(), &first, is_pod_ready()).await?;

    // unseal other pods
    for pod in [1, 2] {
        let mut pf = PodApi::new(pods.clone(), false, "".to_string())
            .http(&format!("{}-{}", &name, pod), VAULT_PORT)
            .await?;

        pf.await_seal_status(is_seal_status_initialized()).await?;

        pf.unseal(&init_result.keys).await?;
    }

    kube::runtime::wait::await_condition(stss.clone(), &name, is_statefulset_ready()).await?;

    Ok(init_result)
}
