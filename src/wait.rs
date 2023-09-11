use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::runtime::wait::Condition;

/// Returns true if the StatefulSet is considered ready.
/// This means that all replicas are available and ready.
#[must_use]
pub fn is_statefulset_ready() -> impl Condition<StatefulSet> {
    |obj: Option<&StatefulSet>| {
        if let Some(sts) = &obj {
            if let Some(status) = &sts.status {
                return match (status.ready_replicas, status.available_replicas) {
                    (Some(ready), Some(available)) => {
                        ready == available && ready == status.replicas
                    }
                    _ => false,
                };
            }
        }
        false
    }
}

/// Returns true if the StatefulSet is considered updated.
/// This means that all replicas are up-to-date.
#[must_use]
pub fn is_statefulset_updated() -> impl Condition<StatefulSet> {
    |obj: Option<&StatefulSet>| {
        if let Some(sts) = &obj {
            if let Some(status) = &sts.status {
                if let Some(updated) = status.updated_replicas {
                    return updated != status.replicas;
                }
            }
        }
        false
    }
}

/// Returns true if the StatefulSet template is using the given version.
#[must_use]
pub fn statefulset_has_version(version: String) -> impl Condition<StatefulSet> {
    move |obj: Option<&StatefulSet>| {
        if let Some(sts) = &obj {
            if let Some(spec) = &sts.spec {
                if let Some(tpl_spec) = &spec.template.spec {
                    return tpl_spec
                        .containers
                        .iter()
                        .filter_map(|c| {
                            if c.name == "vault" {
                                c.image.clone()
                            } else {
                                None
                            }
                        })
                        .all(|image| image.ends_with(&format!(":{}", version)));
                }
            }
        }
        false
    }
}

/// Returns true if the Pod is ready.
#[must_use]
pub fn is_pod_ready() -> impl Condition<Pod> {
    |obj: Option<&Pod>| {
        if let Some(pod) = &obj {
            if let Some(status) = &pod.status {
                if let Some(ref conditions) = status.conditions {
                    return conditions
                        .iter()
                        .any(|c| c.type_ == "Ready" && c.status == "True");
                }
            }
        }
        false
    }
}

/// Returns true if the Pod has the seal status label.
/// This is determined by looking if the `vault-sealed` label exists.
#[must_use]
pub fn is_pod_exporting_seal_status() -> impl Condition<Pod> {
    |obj: Option<&Pod>| {
        if let Some(pod) = &obj {
            if let Some(labels) = &pod.metadata.labels {
                return labels.get("vault-sealed").is_some();
            }
        }
        false
    }
}

/// Returns true if the Pod is unsealed.
/// This is determined by looking at the `vault-sealed` label.
#[must_use]
pub fn is_pod_unsealed() -> impl Condition<Pod> {
    Condition::not(is_pod_sealed())
}

/// Returns true if the Pod is sealed.
/// This is determined by looking at the `vault-sealed` label.
#[must_use]
pub fn is_pod_sealed() -> impl Condition<Pod> {
    |obj: Option<&Pod>| {
        if let Some(pod) = &obj {
            if let Some(labels) = &pod.metadata.labels {
                if let Some(sealed) = labels.get("vault-sealed") {
                    return sealed.as_str() == "true";
                }
            }
        }
        false
    }
}

/// Returns true if the Pod is the active replica.
/// This is determined by looking at the `vault-active` label.
#[must_use]
pub fn is_pod_active() -> impl Condition<Pod> {
    |obj: Option<&Pod>| {
        if let Some(pod) = &obj {
            if let Some(labels) = &pod.metadata.labels {
                if let Some(active) = labels.get("vault-active") {
                    return active.as_str() == "true";
                }
            }
        }
        false
    }
}

/// Returns true if the Pod is a standby replica.
/// This is determined by looking at the `vault-active` label.
#[must_use]
pub fn is_pod_standby() -> impl Condition<Pod> {
    Condition::not(is_pod_active())
}

#[cfg(test)]
mod tests {
    use http::{Request, Response};
    use hyper::Body;
    use k8s_openapi::{
        api::{
            apps::v1::{StatefulSet, StatefulSetStatus},
            core::v1::Pod,
        },
        apimachinery::pkg::apis::meta::v1::WatchEvent,
        List,
    };
    use kube::{Api, Client, ResourceExt};
    use serde_json::Value;
    use tokio_util::sync::CancellationToken;
    use tower_test::mock::{self, Handle};

    use crate::is_statefulset_ready;

    async fn mock_get_pod(handle: &mut Handle<Request<Body>, Response<Body>>) {
        let (request, send) = handle.next_request().await.expect("Service not called");

        let uri = request.uri().to_string();

        let body = match (
            request.method().as_str(),
uri.as_str(),
        ) {
            ("GET", "/apis/apps/v1/namespaces/vault-mgmt-e2e/statefulsets?&fieldSelector=metadata.name%3Dvault-mgmt-e2e-2274&resourceVersion=0") => {
                println!("GET {}", uri);
                let file =
                    tokio::fs::read_to_string(format!("tests/resources/installed/{}.yaml", "apis/apps/v1/namespaces/vault-mgmt-e2e/statefulsets/vault-mgmt-e2e-2274"))
                        .await
                        .unwrap();

                let mut list = List::<StatefulSet>::default();
                list.items.push(serde_yaml::from_str(&file).unwrap());
                list.metadata.resource_version = Some("0".to_string());
                serde_json::to_string(&list).unwrap()
            }
            ("GET", uri) => {
                println!("GET {}", uri);
                let file =
                    tokio::fs::read_to_string(format!("tests/resources/installed/{}.yaml", uri.strip_prefix("/").unwrap()))
                        .await
                        .unwrap();

                serde_json::to_string(&serde_yaml::from_str::<Value>(&file).unwrap()).unwrap()
            }
            _ => panic!("Unexpected API request {:?}", request),
        };

        send.send_response(Response::builder().body(Body::from(body)).unwrap());
    }

    async fn mock_list_sts(
        cancel: CancellationToken,
        handle: &mut Handle<Request<Body>, Response<Body>>,
        states: &[Vec<k8s_openapi::api::apps::v1::StatefulSetStatus>],
    ) {
        let mut i = 1;
        let states = {
            if let Some((last, states)) = states.split_last() {
                states.iter().chain(std::iter::repeat(last))
            } else {
                panic!("no states provided")
            }
        };
        for state_list in states {
            tokio::select! {
                request = handle.next_request() => {
                    let (request, send) = request.expect("Service not called");

                    let method = request.method().to_string();
                    let uri = request.uri().path().to_string();
                    let query = request.uri().query().unwrap().to_string();

                    let watch = query.contains("watch=true");

                    let file = tokio::fs::read_to_string(format!(
                        "tests/resources/installed/{}.yaml",
                        "apis/apps/v1/namespaces/vault-mgmt-e2e/statefulsets/vault-mgmt-e2e-2274"
                    ))
                    .await
                    .unwrap();

                    println!("{} {} {} ", method, uri, query);

                    let body = match (method.as_str(), uri.as_str(), query.as_str(), watch) {
                        ("GET", "/apis/apps/v1/namespaces/vault-mgmt-e2e/statefulsets", _query, false) => {
                            let mut list = List::<StatefulSet>::default();

                            for state in state_list.iter() {
                                let mut sts: StatefulSet = serde_yaml::from_str(&file).unwrap();
                                sts.status = Some(state.clone());
                                list.items.push(sts);
                            }

                            list.metadata.resource_version = Some(format!("{}", i));
                            i += 1;

                            serde_json::to_string(&list).unwrap()
                        }
                        ("GET", "/apis/apps/v1/namespaces/vault-mgmt-e2e/statefulsets", _query, true) => {
                            let mut list = String::new();

                            for state in state_list.iter() {
                                let mut sts: StatefulSet = serde_yaml::from_str(&file).unwrap();
                                sts.status = Some(state.clone());
                                sts.metadata.resource_version = Some(format!("{}", i));
                                i += 1;
                                let event = WatchEvent::Modified(sts);

                                list.push_str(&serde_json::to_string(&event).unwrap());
                                list.push_str("\n");
                            }

                            list
                        }
                        _ => panic!("Unexpected API request {:?} {:?} {:?}", method, uri, query),
                    };

                    send.send_response(Response::builder().body(Body::from(body)).unwrap());
                }
                _ = cancel.cancelled() => {
                    return;
                }
            }
        }
    }

    #[tokio::test]
    async fn test_mock_handle() {
        let (mock_service, mut handle) = mock::pair::<Request<Body>, Response<Body>>();
        let spawned = tokio::spawn(async move {
            mock_get_pod(&mut handle).await;
        });

        let pods: Api<Pod> = Api::default_namespaced(Client::new(mock_service, "vault-mgmt-e2e"));
        let pod = pods.get("vault-mgmt-e2e-2274-0").await.unwrap();
        assert_eq!(pod.labels().get("vault-version").unwrap(), "1.13.0");
        // let pod = pods.get("vault-mgmt-e2e-2274-0").await.unwrap();
        // assert_eq!(pod.labels().get("vault-version").unwrap(), "1.13.0");
        spawned.await.unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_can_succeed_immediately() {
        let (mock_service, mut handle) = mock::pair::<Request<Body>, Response<Body>>();

        let api: Api<StatefulSet> =
            Api::default_namespaced(Client::new(mock_service, "vault-mgmt-e2e"));

        let cancel = CancellationToken::new();
        let cloned_token = cancel.clone();

        let spawned = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            tokio::spawn(async move {
                mock_list_sts(
                    cloned_token,
                    &mut handle,
                    &vec![vec![StatefulSetStatus {
                        replicas: 1,
                        available_replicas: Some(1),
                        ready_replicas: Some(1),
                        current_replicas: Some(1),
                        updated_replicas: Some(1),
                        ..Default::default()
                    }]],
                )
                .await;
            }),
        );

        kube::runtime::wait::await_condition(
            api.clone(),
            "vault-mgmt-e2e-2274",
            is_statefulset_ready(),
        )
        .await
        .unwrap();
        cancel.cancel();

        spawned
            .await
            .expect("timed out waiting for request")
            .unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_does_not_succeed_early() {
        let (mock_service, mut handle) = mock::pair::<Request<Body>, Response<Body>>();

        let api: Api<StatefulSet> =
            Api::default_namespaced(Client::new(mock_service, "vault-mgmt-e2e"));

        let cancel = CancellationToken::new();
        let cloned_token = cancel.clone();

        let spawned = tokio::spawn(async move {
            mock_list_sts(
                cloned_token,
                &mut handle,
                &vec![
                    vec![StatefulSetStatus {
                        replicas: 1,
                        available_replicas: Some(0),
                        ready_replicas: Some(0),
                        current_replicas: Some(0),
                        updated_replicas: Some(0),
                        ..Default::default()
                    }],
                    vec![StatefulSetStatus {
                        replicas: 1,
                        available_replicas: Some(0),
                        ready_replicas: Some(0),
                        current_replicas: Some(1),
                        updated_replicas: Some(1),
                        ..Default::default()
                    }],
                    vec![StatefulSetStatus {
                        replicas: 1,
                        available_replicas: Some(0),
                        ready_replicas: Some(1),
                        current_replicas: Some(1),
                        updated_replicas: Some(1),
                        ..Default::default()
                    }],
                    vec![StatefulSetStatus {
                        replicas: 1,
                        available_replicas: Some(1),
                        ready_replicas: Some(0),
                        current_replicas: Some(1),
                        updated_replicas: Some(1),
                        ..Default::default()
                    }],
                ],
            )
            .await;
        });

        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            tokio::spawn(async move {
                kube::runtime::wait::await_condition(
                    api.clone(),
                    "vault-mgmt-e2e-2274",
                    is_statefulset_ready(),
                )
                .await
                .unwrap();
            }),
        )
        .await
        .expect_err("request should not have completed");
        cancel.cancel();

        spawned.await.unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_waits_correctly() {
        let (mock_service, mut handle) = mock::pair::<Request<Body>, Response<Body>>();

        let api: Api<StatefulSet> =
            Api::default_namespaced(Client::new(mock_service, "vault-mgmt-e2e"));

        let cancel = CancellationToken::new();
        let cloned_token = cancel.clone();

        let spawned = tokio::spawn(async move {
            mock_list_sts(
                cloned_token,
                &mut handle,
                &vec![
                    vec![StatefulSetStatus {
                        replicas: 1,
                        available_replicas: Some(0),
                        ready_replicas: Some(1),
                        current_replicas: Some(0),
                        updated_replicas: Some(0),
                        ..Default::default()
                    }],
                    vec![
                        StatefulSetStatus {
                            replicas: 1,
                            available_replicas: Some(0),
                            ready_replicas: Some(1),
                            current_replicas: Some(1),
                            updated_replicas: Some(1),
                            ..Default::default()
                        },
                        StatefulSetStatus {
                            replicas: 1,
                            available_replicas: Some(1),
                            ready_replicas: Some(1),
                            current_replicas: Some(1),
                            updated_replicas: Some(1),
                            ..Default::default()
                        },
                    ],
                ],
            )
            .await;
        });

        kube::runtime::wait::await_condition(
            api.clone(),
            "vault-mgmt-e2e-2274",
            is_statefulset_ready(),
        )
        .await
        .unwrap();
        cancel.cancel();

        spawned.await.unwrap();
    }
}
