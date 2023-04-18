#[macro_use]
extern crate prettytable;

use clap::ValueEnum;
use futures_util::StreamExt;
use hyper::{Body, Request};
use hyper_rustls::ConfigBuilderExt;
use k8s_openapi::api::{apps::v1::StatefulSet, core::v1::Pod};
use kube::{
    api::{Api, AttachParams, AttachedProcess, DeleteParams, ListParams},
    runtime::wait::{conditions::is_pod_running, Condition},
};
use secrecy::{ExposeSecret, Secret};
use std::{collections::HashMap, io::Write, sync::Arc};
use tokio::{io::AsyncWriteExt, process::Command};
use tower::ServiceExt;
use tracing::*;

mod show;
pub use show::show;

#[derive(ValueEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExecIn {
    Active,
    Standby,
    Sealed,
}

impl std::fmt::Display for ExecIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
}

impl ExecIn {
    pub fn to_label_selector(&self) -> &str {
        match self {
            ExecIn::Active => "vault-active=true",
            ExecIn::Standby => "vault-active=false",
            ExecIn::Sealed => "vault-sealed=true",
        }
    }
}

#[tracing::instrument(skip_all, fields(cmd, exec_in = %exec_in))]
pub async fn exec(
    api: &Api<Pod>,
    cmd: String,
    exec_in: ExecIn,
    env: HashMap<String, Secret<String>>,
) -> anyhow::Result<()> {
    let pods = api
        .list(
            &ListParams::default()
                .labels("app.kubernetes.io/name=vault")
                .labels(exec_in.to_label_selector()),
        )
        .await?;
    let pod = pods
        .items
        .first()
        .ok_or(anyhow::anyhow!("no matching vault pod found"))?;

    let (stdout, stderr) = exec_pod(&api, pod, cmd, env).await?;

    std::io::stdout().write_all(stdout.as_bytes())?;
    std::io::stderr().write_all(stderr.as_bytes())?;

    Ok(())
}

#[tracing::instrument(skip_all, fields(domain, pod = %pod.metadata.name.as_ref().unwrap()))]
pub async fn step_down(
    domain: &str,
    api: &Api<Pod>,
    pod: &Pod,
    token: Secret<String>,
) -> anyhow::Result<()> {
    let mut sender = https_forward(domain, api, pod).await?;

    let http_req = Request::builder()
        .uri("/v1/sys/step-down")
        .header("Connection", "close")
        .header("Host", "127.0.0.1")
        .header("X-Vault-Request", "true")
        .header("X-Vault-Token", token.expose_secret())
        .method(hyper::Method::PUT)
        .body(Body::from(""))?;

    let (parts, body) = sender.send_request(http_req).await?.into_parts();

    let body = hyper::body::to_bytes(body).await?;
    let body = String::from_utf8(body.to_vec())?;

    if parts.status != hyper::StatusCode::NO_CONTENT {
        return Err(anyhow::anyhow!("{}", body));
    }

    debug!("step down successful");

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn wait_until_ready(api: &Api<StatefulSet>, name: String) -> anyhow::Result<()> {
    debug!("waiting for statefulset to be ready: {}", name);

    kube::runtime::wait::await_condition(api.clone(), &name, |obj: Option<&StatefulSet>| {
        if let Some(sts) = &obj {
            if let Some(status) = &sts.status {
                if let Some(ready) = status.available_replicas {
                    if ready != status.replicas {
                        return false;
                    }
                }
                if let Some(ready) = status.ready_replicas {
                    if ready != status.replicas {
                        return false;
                    }
                }
                if let Some(ready) = status.updated_replicas {
                    if ready != status.replicas {
                        return false;
                    }
                }

                return true;
            }
        }
        false
    })
    .await?;

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn wait_until_running(api: &Api<StatefulSet>, name: String) -> anyhow::Result<()> {
    debug!("waiting for statefulset to be running: {}", name);

    kube::runtime::wait::await_condition(api.clone(), &name, |obj: Option<&StatefulSet>| {
        if let Some(sts) = &obj {
            if let Some(status) = &sts.status {
                if let Some(ready) = status.updated_replicas {
                    if ready != status.replicas {
                        return false;
                    }
                }

                return true;
            }
        }
        false
    })
    .await?;

    Ok(())
}

#[tracing::instrument()]
pub async fn get_unseal_keys(key_cmd: String) -> anyhow::Result<Vec<Secret<String>>> {
    let output = Command::new("sh").arg("-c").arg(key_cmd).output().await?;

    let stdout = String::from_utf8(output.stdout)?;
    let keys = stdout
        .lines()
        .collect::<Vec<_>>()
        .iter()
        .map(|k| Secret::new(k.to_string()))
        .collect();

    Ok(keys)
}

#[tracing::instrument(skip_all)]
pub async fn unseal(domain: &str, api: &Api<Pod>, keys: Vec<Secret<String>>) -> anyhow::Result<()> {
    let pods = api
        .list(
            &ListParams::default()
                .labels("app.kubernetes.io/name=vault")
                .labels(ExecIn::Sealed.to_label_selector()),
        )
        .await?;

    if pods.items.len() == 0 {
        info!("no sealed pods found");
        return Ok(());
    }

    info!("trying {} keys", keys.len());

    for pod in pods.iter() {
        info!(
            "unsealing: {}",
            pod.metadata
                .name
                .clone()
                .ok_or(anyhow::anyhow!("pod does not have a name"))?
        );

        let mut sender = https_forward(domain, api, pod).await?;

        for key in &keys {
            sender.ready().await?;

            let body = serde_json::json!({
                "key": key.expose_secret(),
                "reset": false,
                "migrate": false,
            });

            let http_req = Request::builder()
                .uri("/v1/sys/unseal")
                .header("Host", "127.0.0.1")
                .header("X-Vault-Request", "true")
                .method(hyper::Method::PUT)
                .body(Body::from(body.to_string()))?;

            let (parts, body) = sender.send_request(http_req).await?.into_parts();

            let body = hyper::body::to_bytes(body).await?;
            let body = String::from_utf8(body.to_vec())?;

            if parts.status != hyper::StatusCode::OK {
                return Err(anyhow::anyhow!("{}", body));
            }
        }
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn upgrade(
    domain: &str,
    stss: &Api<StatefulSet>,
    pods: &Api<Pod>,
    token: Secret<String>,
    keys: Vec<Secret<String>>,
) -> anyhow::Result<()> {
    let sts = stss.get("vault").await?;

    let standby = pods
        .list(
            &ListParams::default()
                .labels("app.kubernetes.io/name=vault")
                .labels(ExecIn::Standby.to_label_selector()),
        )
        .await?;

    if standby.items.len() == 0 {
        info!("no standby pods found, skipping upgrade");
        return Ok(());
    }

    let active = pods
        .list(
            &ListParams::default()
                .labels("app.kubernetes.io/name=vault")
                .labels(ExecIn::Active.to_label_selector()),
        )
        .await?;

    if active.items.len() == 0 {
        info!("no active pods found, skipping upgrade");
        return Ok(());
    }

    info!("upgrading standby pods");
    for pod in standby.iter() {
        wait_until_ready(
            stss,
            sts.metadata
                .name
                .clone()
                .ok_or(anyhow::anyhow!("statefulset does not have a name"))?,
        )
        .await?;

        let name = pod
            .metadata
            .name
            .clone()
            .ok_or(anyhow::anyhow!("pod does not have a name"))?;
        info!("upgrading pod: {}", name);

        pods.delete(&name, &DeleteParams::default())
            .await?
            .map_left(|_| info!("deleting pod: {}", name))
            .map_right(|_| info!("deleted pod: {}", name));

        debug!("waiting for pod to be deleted: {}", name);
        kube::runtime::wait::await_condition(pods.clone(), &name, Condition::not(is_pod_running()))
            .await?;

        debug!("waiting for pod to be running: {}", name);
        kube::runtime::wait::await_condition(pods.clone(), &name, is_pod_running()).await?;

        unseal(domain, pods, keys.clone()).await?;
    }

    info!("upgrading currently active pods");
    for pod in active.iter() {
        wait_until_ready(
            stss,
            sts.metadata
                .name
                .clone()
                .ok_or(anyhow::anyhow!("statefulset does not have a name"))?,
        )
        .await?;

        let name = pod
            .metadata
            .name
            .clone()
            .ok_or(anyhow::anyhow!("pod does not have a name"))?;
        info!("upgrading pod: {}", name);

        info!("stepping down: {}", name);
        step_down(domain, pods, pod, token.clone()).await?;

        pods.delete(&name, &DeleteParams::default())
            .await?
            .map_left(|_| info!("deleting pod: {}", name))
            .map_right(|_| info!("deleted pod: {}", name));

        debug!("waiting for pod to be deleted: {}", name);
        kube::runtime::wait::await_condition(pods.clone(), &name, Condition::not(is_pod_running()))
            .await?;

        debug!("waiting for pod to be running: {}", name);
        kube::runtime::wait::await_condition(pods.clone(), &name, is_pod_running()).await?;

        unseal(domain, pods, keys.clone()).await?;
    }

    wait_until_ready(
        stss,
        sts.metadata
            .name
            .clone()
            .ok_or(anyhow::anyhow!("statefulset does not have a name"))?,
    )
    .await?;

    Ok(())
}

#[tracing::instrument(
    skip_all,
    fields(pod = %pod.metadata.name.clone().ok_or(anyhow::anyhow!("pod does not have a name"))?,
    cmd = %cmd,
    env_vars = ?env.keys()),
)]
pub async fn exec_pod(
    api: &Api<Pod>,
    pod: &Pod,
    cmd: String,
    env: HashMap<String, Secret<String>>,
) -> anyhow::Result<(String, String)> {
    let mut attached = api
        .exec(
            &pod.metadata
                .name
                .clone()
                .ok_or(anyhow::anyhow!("pod does not have a name"))?,
            vec!["sh"],
            &AttachParams::default().stdin(true),
        )
        .await?;

    let mut stdin_writer = attached
        .stdin()
        .ok_or(anyhow::anyhow!("no stdin available"))?;

    let mut cmd_with_env_vars = String::new();
    for (k, v) in env {
        cmd_with_env_vars.push_str(&format!("{}={} ", k, v.expose_secret()));
    }
    cmd_with_env_vars.push_str(&cmd);
    cmd_with_env_vars.push_str("\nexit\n");

    stdin_writer.write_all(cmd_with_env_vars.as_bytes()).await?;

    let (stdout, stderr) = get_output(attached).await?;

    Ok((stdout, stderr))
}

#[tracing::instrument(skip_all)]
pub async fn get_output(mut attached: AttachedProcess) -> anyhow::Result<(String, String)> {
    let stdout = tokio_util::io::ReaderStream::new(
        attached
            .stdout()
            .ok_or(anyhow::anyhow!("no stdout available"))?,
    );
    let stderr = tokio_util::io::ReaderStream::new(
        attached
            .stderr()
            .ok_or(anyhow::anyhow!("no stderr available"))?,
    );
    attached.join().await?;
    let out = stdout
        .filter_map(|r| async { r.ok().and_then(|v| String::from_utf8(v.to_vec()).ok()) })
        .collect::<Vec<_>>()
        .await
        .join("");
    let err = stderr
        .filter_map(|r| async { r.ok().and_then(|v| String::from_utf8(v.to_vec()).ok()) })
        .collect::<Vec<_>>()
        .await
        .join("");
    Ok((out, err))
}

#[tracing::instrument(skip_all, fields(domain, pod = %pod.metadata.name.as_ref().unwrap()))]
pub async fn https_forward(
    domain: &str,
    api: &Api<Pod>,
    pod: &Pod,
) -> anyhow::Result<hyper::client::conn::SendRequest<Body>> {
    trace!("forwarding port to pod");

    let mut pf = api
        .portforward(
            pod.metadata
                .name
                .clone()
                .ok_or(anyhow::anyhow!("pod does not have a name"))?
                .as_str(),
            &[8200],
        )
        .await?;
    let port = pf
        .take_stream(8200)
        .ok_or(anyhow::anyhow!("port 8200 is not available"))?;

    let tls = tokio_rustls::rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_native_roots()
        .with_no_client_auth();

    let tls_stream = tokio_rustls::TlsConnector::from(Arc::new(tls))
        .connect(tokio_rustls::rustls::ServerName::try_from(domain)?, port)
        .await?;

    let (sender, connection) = hyper::client::conn::handshake(tls_stream).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            warn!("Error in connection: {}", e);
        }
    });

    Ok(sender)
}
