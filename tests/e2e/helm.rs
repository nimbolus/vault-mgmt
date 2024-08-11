use std::process::Stdio;
use tokio::{io::AsyncWriteExt, process::Command};

pub async fn add_repo() -> anyhow::Result<String> {
    let helm = which::which("helm")?;

    let cmd = Command::new(helm)
        .args([
            "repo",
            "add",
            "hashicorp",
            "https://helm.releases.hashicorp.com",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let output = cmd.wait_with_output().await?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    match output.status.success() {
        false if stdout.contains("already exists with the same configuration") => {
            Err(anyhow::anyhow!("failed to add helm repo: {}", stderr))
        }
        _ => Ok(stdout),
    }
}

pub async fn install_chart(
    namespace: &str,
    name: &str,
    version: Option<&str>,
) -> anyhow::Result<String> {
    let helm = which::which("helm")?;

    let fullname_override = format!("fullnameOverride={}", name);

    let mut args = vec![
        "upgrade",
        "--install",
        name,
        "hashicorp/vault",
        "--namespace",
        namespace,
        "-f",
        "-",
        "--set",
        &fullname_override,
    ];

    let vers;

    if let Some(version) = version {
        vers = format!("server.image.tag={}", version);
        args.push("--set");
        args.push(&vers);
    }

    let mut cmd = Command::new(helm)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = cmd.stdin.take().expect("failed to take stdin");
    tokio::spawn(async move {
        stdin
            .write_all(include_str!("helm/values.yaml").as_bytes())
            .await
            .unwrap();
    });

    let output = cmd.wait_with_output().await?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    match output.status.success() {
        true => Ok(stdout),
        false => Err(anyhow::anyhow!("failed to install helm chart: {}", stderr)),
    }
}

pub async fn uninstall_chart(namespace: &str, name: &str) -> anyhow::Result<String> {
    let helm = which::which("helm")?;

    let cmd = Command::new(helm)
        .args(["uninstall", name, "--namespace", namespace])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let output = cmd.wait_with_output().await?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    match output.status.success() {
        true => Ok(stdout),
        false => Err(anyhow::anyhow!(
            "failed to uninstall helm chart: {}",
            stderr
        )),
    }
}
