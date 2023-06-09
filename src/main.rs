use clap::builder::TypedValueParser;
use clap::{ArgAction, Parser, Subcommand};
use k8s_openapi::api::apps::v1::StatefulSet;
use kube::{api::Api, core::ObjectMeta, Client};
use secrecy::Secret;
use std::collections::HashMap;
use std::str::FromStr;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};
use vault_mgmt::{GetUnsealKeys, GetUnsealKeysFromVault};

use vault_mgmt::{
    construct_table, is_statefulset_ready, StepDown, VAULT_PORT, {self, exec, ExecIn},
    {get_unseal_keys, list_sealed_pods, Unseal}, {list_vault_pods, PodApi, StatefulSetApi},
};

/// Manage your vault installation in Kubernetes
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Namespace to work in
    #[arg(short, long, default_value = "vault")]
    namespace: String,

    /// Log level
    #[arg(
        short,
        long,
        default_value = "info",
        value_parser = clap::builder::PossibleValuesParser::new(
            ["error", "warn", "info", "debug", "trace"]
        ).map(|s| tracing::Level::from_str(&s).expect("invalid log level")),
    )]
    log_level: tracing::Level,

    /// Statefulset name
    #[arg(long, default_value = "vault")]
    statefulset: String,

    /// Vault domain name, used for TLS verification
    #[arg(long, default_value = "vault")]
    domain: String,

    /// Use TLS for communication with vault
    #[arg(long, action = ArgAction::Set, default_value = "true")]
    tls: bool,

    /// Verify TLS certificate
    #[arg(long, action = ArgAction::Set, default_value = "true")]
    tls_verify: bool,

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
        /// vault token to use for retrieving the unseal keys
        /// if not provided, the token will be read from the VAULT_TOKEN environment variable
        #[arg(short, long)]
        token: Option<Secret<String>>,

        /// uri to vault kv secret containing the unseal keys.
        /// for example: `https://vault.example.com/v1/secret/data/vault/unseal-keys`.
        /// the secret must store the keys separated by newlines in the data field `keys`.
        #[arg(long)]
        keys_secret_uri: Option<String>,

        /// command that writes unseal keys to its stdout.
        /// each line will be used as a key.
        /// the command will be executed locally
        #[arg(long)]
        key_cmd: Option<String>,
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
        /// vault token to use for the step down (and retrieving the unseal keys if configured)
        /// if not provided, the token will be read from the VAULT_TOKEN environment variable
        #[arg(short, long)]
        token: Option<Secret<String>>,

        /// Unseal the pods after upgrading.
        /// If this is set to false, the upgrade process will wait for the pods to be unsealed externally.
        #[arg(short = 'u', long, action = ArgAction::Set, default_value = "true")]
        should_unseal: bool,

        /// Force upgrading the pods even when the version is already updated.
        /// If this is not enabled, every upgraded pod will be skipped.
        /// This is useful when you want to roll the pods gracefully for other reasons (e.g. certificate rotation).
        #[arg(short, long, action = ArgAction::Set, default_value = "false")]
        force_upgrade: bool,

        /// uri to vault kv secret containing the unseal keys.
        /// for example: `https://vault.example.com/v1/secret/data/vault/unseal-keys`.
        /// the secret must store the keys separated by newlines in the data field `keys`.
        #[arg(long)]
        keys_secret_uri: Option<String>,

        /// command that writes unseal keys to its stdout.
        /// each line will be used as a key.
        /// the command will be executed locally
        #[arg(long)]
        key_cmd: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("vault_mgmt={}", cli.log_level)));
    tracing::subscriber::set_global_default(
        Registry::default()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer()),
    )?;

    match cli.command {
        Commands::Show {} => {
            let api = setup_api(&cli.namespace).await?;
            let table = construct_table(&api).await?;

            table.printstd();
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
                .list(&list_vault_pods().labels(&ExecIn::Active.to_label_selector()))
                .await?;
            let active = active.iter().next().ok_or(anyhow::anyhow!(
                "no active vault pod found. is vault sealed?"
            ))?;

            PodApi::new(api, cli.tls, cli.domain)
                .http(
                    active
                        .metadata
                        .name
                        .as_ref()
                        .ok_or(anyhow::anyhow!("pod does not have a name"))?
                        .as_str(),
                    VAULT_PORT,
                )
                .await?
                .step_down(get_token(token)?)
                .await?;
        }
        Commands::WaitUntilReady {} => {
            let api: Api<StatefulSet> = setup_api(&cli.namespace).await?;
            kube::runtime::wait::await_condition(
                api.clone(),
                &cli.statefulset,
                is_statefulset_ready(),
            )
            .await?;
        }
        Commands::Unseal {
            token,
            keys_secret_uri,
            key_cmd,
        } => {
            let api = setup_api(&cli.namespace).await?;
            let sealed = list_sealed_pods(&api).await?;

            if sealed.is_empty() {
                return Ok(());
            }

            let mut keys = Vec::new();

            if let Some(path) = keys_secret_uri {
                let token = get_token(token)?;

                let uri = http::Uri::from_str(&path)?;

                let mut client = GetUnsealKeysFromVault::new(&uri)?;

                let mut k = client
                    .get_unseal_keys(
                        uri.path_and_query()
                            .ok_or(anyhow::anyhow!("keys secret uri is not valid: {}", path))?,
                        token,
                    )
                    .await?;

                keys.append(&mut k);
            } else if let Some(cmd) = key_cmd {
                let mut k = get_unseal_keys(&cmd).await?;

                if k.is_empty() {
                    anyhow::bail!("no unseal keys returned from command")
                }

                keys.append(&mut k);
            } else {
                anyhow::bail!("no keys secret uri or key cmd specified")
            }

            for pod in sealed.iter() {
                PodApi::new(api.clone(), cli.tls, cli.domain.clone())
                    .http(
                        pod.metadata
                            .name
                            .as_ref()
                            .ok_or(anyhow::anyhow!("pod does not have a name"))?
                            .as_str(),
                        VAULT_PORT,
                    )
                    .await?
                    .unseal(&keys)
                    .await?;
            }
        }
        Commands::Upgrade {
            token,
            should_unseal,
            force_upgrade,
            keys_secret_uri,
            key_cmd,
        } => {
            let stss = setup_api(&cli.namespace).await?;
            let pods = setup_api(&cli.namespace).await?;

            let mut keys = Vec::new();

            let token = get_token(token)?;

            if let Some(path) = keys_secret_uri {
                let uri = http::Uri::from_str(&path)?;

                let mut client = GetUnsealKeysFromVault::new(&uri)?;

                let mut k = client
                    .get_unseal_keys(
                        uri.path_and_query()
                            .ok_or(anyhow::anyhow!("keys secret uri is not valid: {}", path))?,
                        token.clone(),
                    )
                    .await?;

                keys.append(&mut k);
            } else if let Some(cmd) = key_cmd {
                let mut k = get_unseal_keys(&cmd).await?;

                if k.is_empty() {
                    anyhow::bail!("no unseal keys returned from command")
                }

                keys.append(&mut k);
            } else {
                anyhow::bail!("no keys secret uri or key cmd specified")
            }

            let sts = stss.get(&cli.statefulset).await?;

            StatefulSetApi::from(stss.clone())
                .upgrade(
                    sts.clone(),
                    &PodApi::new(pods.clone(), cli.tls, cli.domain),
                    token,
                    should_unseal,
                    force_upgrade,
                    &keys,
                )
                .await?;

            kube::runtime::wait::await_condition(
                stss.clone(),
                &sts.metadata
                    .name
                    .clone()
                    .ok_or(anyhow::anyhow!("statefulset does not have a name"))?,
                is_statefulset_ready(),
            )
            .await?;
        }
    }

    Ok(())
}

fn get_token(arg: Option<Secret<String>>) -> anyhow::Result<Secret<String>> {
    match arg {
        Some(token) => Ok(token),
        None => Ok(std::env::var("VAULT_TOKEN")
            .map_err(|_| anyhow::anyhow!("neither VAULT_TOKEN nor --token specified"))?
            .into()),
    }
}

fn collect_env(
    env_pairs: Vec<String>,
    env_var_keys: Vec<String>,
) -> anyhow::Result<HashMap<String, Secret<String>>> {
    let mut env = from_env(env_var_keys)?;

    for e in env_pairs {
        let mut split = e.split('=');
        let k = split
            .next()
            .ok_or(anyhow::anyhow!("invalid key=value pair"))?;
        let v = split
            .next()
            .ok_or(anyhow::anyhow!("invalid key=value pair"))?;
        env.insert(k.to_string(), Secret::from(v.to_string()));
    }

    Ok(env)
}

fn from_env(env_var_keys: Vec<String>) -> anyhow::Result<HashMap<String, Secret<String>>> {
    let mut env = HashMap::new();
    for key in env_var_keys {
        let value = std::env::var(&key).map_err(|_| anyhow::anyhow!(format!("{} not set", key)))?;
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
