use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use prettytable::{color, Attr, Cell, Row, Table};

use crate::list_vault_pods;

#[tracing::instrument(skip_all)]
pub async fn construct_table(api: &Api<Pod>) -> anyhow::Result<Table> {
    let mut table = Table::new();
    table.set_titles(row![
        "NAME",
        "STATUS",
        "IMAGE",
        "INITIALIZED",
        "SEALED",
        "ACTIVE",
        "READY",
    ]);

    let pods = api.list(&list_vault_pods()).await?;

    let get_vault_label = |pod: &Pod, label: &str| match pod.metadata.labels {
        Some(ref labels) => labels
            .get(label)
            .unwrap_or(&String::from("unknown"))
            .to_string(),
        None => String::from("unknown"),
    };

    for p in pods.iter() {
        let name = p
            .metadata
            .name
            .clone()
            .ok_or(anyhow::anyhow!("pod does not have a name"))?;
        let status = p
            .status
            .as_ref()
            .ok_or(anyhow::anyhow!("pod does not have a status"))?
            .phase
            .clone()
            .ok_or(anyhow::anyhow!("pod does not have a phase"))?;
        let image = p
            .spec
            .as_ref()
            .ok_or(anyhow::anyhow!("pod does not have a spec"))?
            .containers
            .first()
            .ok_or(anyhow::anyhow!("pod does not have a container"))?
            .image
            .clone()
            .ok_or(anyhow::anyhow!("container does not have an image"))?;

        let initialized = get_vault_label(p, "vault-initialized");
        let initialized =
            Cell::new(&initialized).with_style(Attr::ForegroundColor(match initialized.as_str() {
                "true" => color::GREEN,
                "false" => color::RED,
                _ => color::YELLOW,
            }));

        let sealed = get_vault_label(p, "vault-sealed");
        let sealed = Cell::new(&sealed).with_style(Attr::ForegroundColor(match sealed.as_str() {
            "true" => color::RED,
            "false" => color::GREEN,
            _ => color::YELLOW,
        }));

        let active = get_vault_label(p, "vault-active");
        let active = Cell::new(&active).with_style(Attr::ForegroundColor(match active.as_str() {
            "true" => color::GREEN,
            "false" => color::WHITE,
            _ => color::YELLOW,
        }));

        let ready = {
            let mut ready = "unknown".to_string();

            for c in p
                .status
                .as_ref()
                .ok_or(anyhow::anyhow!("pod does not have a status"))?
                .conditions
                .as_ref()
                .ok_or(anyhow::anyhow!("pod does not have status conditions"))?
            {
                if c.type_ == "Ready" {
                    ready = match c.status.as_str() {
                        "True" => "true".to_string(),
                        "False" => "false".to_string(),
                        _ => "unknown".to_string(),
                    };

                    break;
                }
            }

            ready
        };
        let ready = Cell::new(&ready).with_style(Attr::ForegroundColor(match ready.as_str() {
            "true" => color::GREEN,
            "false" => color::WHITE,
            _ => color::YELLOW,
        }));

        table.add_row(Row::new(vec![
            Cell::new(&name),
            Cell::new(&status),
            Cell::new(&image),
            initialized,
            sealed,
            active,
            ready,
        ]));
    }

    Ok(table)
}
