use clap::builder::TypedValueParser;
use clap::{Parser, Subcommand};
use k8s_openapi::api::apps::v1::StatefulSet;
use kube::{
    api::{Api, ListParams},
    core::ObjectMeta,
    Client,
};
use secrecy::Secret;
use std::collections::HashMap;
use std::str::FromStr;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};

use vault_mgmt::*;

/// Manage your vault installation in Kubernetes
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Namespace to work in
    #[arg(short, long, default_value = "vault")]
    namespace: String,

    /// Log level
    #[arg(short, long, default_value = "info", value_parser = clap::builder::PossibleValuesParser::new(["error", "warn", "info", "debug", "trace"]).map(|s| tracing::Level::from_str(&s).unwrap()))]
    log_level: tracing::Level,

    /// Vault domain name, used for TLS verification
    domain: String,

    /// Subcommand to run
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
#[command(arg_required_else_help = true)]
enum Commands {
    /// Show the current state of the vault pods
    Show {},

    /// Execute a command in the vault pod
    #[command(arg_required_else_help = true)]
    Exec {
        /// command to execute
        cmd: Vec<String>,

        /// where to run the command
        #[arg(
            short = 'i',
            long = "in",
            value_name = "IN",
            default_value_t = ExecIn::Active,
            value_enum
        )]
        exec_in: ExecIn,

        /// environment variables to set as key=value pairs
        #[arg(short, long)]
        env: Vec<String>,

        /// environment variables to set from the current environment
        #[arg(short = 'k', long)]
        env_keys: Vec<String>,
    },

    /// Unseal all sealed pods
    #[command(arg_required_else_help = true)]
    Unseal {
        /// command that writes unseal keys to its stdout.
        /// each line will be used as a key.
        /// the command will be executed locally
        key_cmd: Vec<String>,
    },

    /// Step down the active pod
    StepDown {
        /// vault token to use for the step down
        /// if not provided, the token will be read from the VAULT_TOKEN environment variable
        #[arg(short, long)]
        token: Option<Secret<String>>,
    },

    /// Wait until the statefulset is ready
    WaitUntilReady {},

    /// Do a rolling upgrade of the vault pods without downtime
    ///
    /// This will upgrade the standby pods first by deleting the pods and them getting recreated
    /// by the statefulset automatically. After the upgrade is complete, we step down the active pod.
    /// After a standby pod having taken over, the previously active pod is upgraded.
    #[command(arg_required_else_help = true)]
    Upgrade {
        /// vault token to use for the step down
        /// if not provided, the token will be read from the VAULT_TOKEN environment variable
        #[arg(short, long)]
        token: Option<Secret<String>>,

        /// command that writes unseal keys to its stdout.
        /// each line will be used as a key.
        /// the command will be executed locally
        key_cmd: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("vault_mgmt={}", cli.log_level.to_string())));
    tracing::subscriber::set_global_default(
        Registry::default()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer()),
    )
    .expect("Failed to set tracing subscriber");

    match cli.command {
        Commands::Show {} => {
            let api = setup_api(&cli.namespace).await?;
            show(&api).await?;
        }
        Commands::Exec {
            cmd,
            exec_in,
            env,
            env_keys,
        } => {
            let api = setup_api(&cli.namespace).await?;
            let env = collect_env(env, env_keys)?;
            exec(&api, cmd.join(" "), exec_in, env).await?;
        }
        Commands::StepDown { token } => {
            let api = setup_api(&cli.namespace).await?;
            let active = api
                .list(
                    &ListParams::default()
                        .labels("app.kubernetes.io/name=vault")
                        .labels(ExecIn::Active.to_label_selector()),
                )
                .await?;
            let active = active.iter().next().expect("no active pod found");

            step_down(
                &cli.domain,
                &api,
                active,
                token.unwrap_or_else(|| {
                    std::env::var("VAULT_TOKEN")
                        .expect("VAULT_TOKEN not set")
                        .into()
                }),
            )
            .await?;
        }
        Commands::WaitUntilReady {} => {
            let api: Api<StatefulSet> = setup_api(&cli.namespace).await?;
            wait_until_ready(&api, "vault".to_owned()).await?;
        }
        Commands::Unseal { key_cmd } => {
            let api = setup_api(&cli.namespace).await?;
            let keys = get_unseal_keys(key_cmd.join(" ")).await?;
            if keys.len() == 0 {
                anyhow::bail!("no unseal keys returned from command")
            }
            unseal(&cli.domain, &api, keys).await?;
        }
        Commands::Upgrade { token, key_cmd } => {
            let sts = setup_api(&cli.namespace).await?;
            let pods = setup_api(&cli.namespace).await?;
            let keys = get_unseal_keys(key_cmd.join(" ")).await?;
            if keys.len() == 0 {
                anyhow::bail!("no unseal keys returned from command")
            }
            upgrade(
                &cli.domain,
                &sts,
                &pods,
                token.unwrap_or_else(|| {
                    std::env::var("VAULT_TOKEN")
                        .expect("VAULT_TOKEN not set")
                        .into()
                }),
                keys,
            )
            .await?;
        }
    }

    Ok(())
}

fn collect_env(
    env_pairs: Vec<String>,
    env_var_keys: Vec<String>,
) -> anyhow::Result<HashMap<String, Secret<String>>> {
    let mut env = from_env(env_var_keys)?;

    for e in env_pairs {
        let mut split = e.split("=");
        let k = split.next().unwrap();
        let v = split.next().unwrap();
        env.insert(k.to_string(), Secret::from(v.to_string()));
    }

    Ok(env)
}

fn from_env(env_var_keys: Vec<String>) -> anyhow::Result<HashMap<String, Secret<String>>> {
    let mut env = HashMap::new();
    for key in env_var_keys {
        let value = std::env::var(&key).expect(&format!("{} not set", key));
        env.insert(key, Secret::new(value));
    }
    Ok(env)
}

async fn setup_api<T>(namespace: &str) -> anyhow::Result<Api<T>>
where
    T: k8s_openapi::Metadata<Ty = ObjectMeta>,
    T: k8s_openapi::Resource<Scope = k8s_openapi::NamespaceResourceScope>,
{
    let client = Client::try_default().await?;

    let pods: Api<T> = Api::namespaced(client, namespace);

    Ok(pods)
}
