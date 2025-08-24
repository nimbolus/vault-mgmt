use clap::ValueEnum;
use futures_util::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams, AttachedProcess};
use secrecy::{ExposeSecret, Secret};
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;

use crate::{list_vault_pods, Flavor};

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
    pub fn to_label_selector(&self, flavor: &str) -> String {
        match self {
            ExecIn::Active => format!("{}=true", &format!("{}-active", flavor)),
            ExecIn::Standby => format!("{}=false", &format!("{}-active", flavor)),
            ExecIn::Sealed => format!("{}=true", &format!("{}-sealed", flavor)),
        }
    }
}

#[tracing::instrument(skip_all, fields(cmd, exec_in = %exec_in))]
pub async fn exec(
    api: &Api<Pod>,
    cmd: String,
    exec_in: ExecIn,
    flavor: Flavor,
    env: HashMap<String, Secret<String>>,
) -> anyhow::Result<()> {
    let pods = api
        .list(
            &list_vault_pods(&flavor.to_string())
                .labels(&exec_in.to_label_selector(&flavor.to_string())),
        )
        .await?;
    let pod = pods
        .items
        .first()
        .ok_or(anyhow::anyhow!("no matching vault pod found"))?;

    let (stdout, stderr) = exec_pod(api, pod, cmd, env).await?;

    tokio::io::stdout().write_all(stdout.as_bytes()).await?;
    tokio::io::stderr().write_all(stderr.as_bytes()).await?;

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
