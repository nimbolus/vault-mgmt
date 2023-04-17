// separate file to not include the OwoColorize methods everywhere

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use owo_colors::{AnsiColors, DynColors, OwoColorize, Stream::Stdout};
use prettytable::Table;

#[tracing::instrument(skip_all)]
pub async fn show(api: &Api<Pod>) -> anyhow::Result<()> {
    let mut table = Table::new();
    table.add_row(row![
        "NAME".if_supports_color(Stdout, |text| text.bold()),
        "STATUS".if_supports_color(Stdout, |text| text.bold()),
        "IMAGE".if_supports_color(Stdout, |text| text.bold()),
        "INITIALIZED".if_supports_color(Stdout, |text| text.bold()),
        "ACTIVE".if_supports_color(Stdout, |text| text.bold()),
        "SEALED".if_supports_color(Stdout, |text| text.bold()),
        "READY".if_supports_color(Stdout, |text| text.bold()),
    ]);

    let pods = api
        .list(&ListParams::default().labels("app.kubernetes.io/name=vault"))
        .await?;

    let get_vault_label = |pod: &Pod, label: &str| {
        pod.metadata
            .labels
            .as_ref()
            .unwrap()
            .get(label)
            .map_or(String::from("unknown"), |s| s.to_string())
    };

    for p in pods.iter() {
        let name = p.metadata.name.clone().unwrap();
        let status = p.status.as_ref().unwrap().phase.clone().unwrap();
        let image = p.spec.as_ref().unwrap().containers[0]
            .image
            .clone()
            .unwrap();
        let initialized = get_vault_label(p, "vault-initialized");
        let initialized = initialized.if_supports_color(Stdout, |text| {
            text.color(match initialized.as_str() {
                "true" => DynColors::Ansi(AnsiColors::Green),
                "false" => DynColors::Ansi(AnsiColors::Red),
                _ => DynColors::Ansi(AnsiColors::Yellow),
            })
        });
        let active = get_vault_label(p, "vault-active");
        let active = active.if_supports_color(Stdout, |text| {
            text.color(match active.as_str() {
                "true" => DynColors::Ansi(AnsiColors::Green),
                "false" => DynColors::Ansi(AnsiColors::White),
                _ => DynColors::Ansi(AnsiColors::Yellow),
            })
        });
        let sealed = get_vault_label(p, "vault-sealed");
        let sealed = sealed.if_supports_color(Stdout, |text| {
            text.color(match sealed.as_str() {
                "true" => DynColors::Ansi(AnsiColors::Red),
                "false" => DynColors::Ansi(AnsiColors::Green),
                _ => DynColors::Ansi(AnsiColors::Yellow),
            })
        });
        let ready = {
            let mut ready = "unknown".to_string();

            for c in p.status.as_ref().unwrap().conditions.as_ref().unwrap() {
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
        let ready = ready.if_supports_color(Stdout, |text| {
            text.color(match ready.as_str() {
                "true" => DynColors::Ansi(AnsiColors::Green),
                "false" => DynColors::Ansi(AnsiColors::White),
                _ => DynColors::Ansi(AnsiColors::Yellow),
            })
        });

        table.add_row(row![
            name,
            status,
            image,
            initialized,
            active,
            sealed,
            ready,
        ]);
    }

    table.printstd();

    Ok(())
}
