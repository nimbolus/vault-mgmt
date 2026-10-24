# vault-mgmt

Manage [Vault](https://www.vaultproject.io/) and [OpenBao](https://openbao.org/) clusters running in Kubernetes: inspect their state, unseal pods, step down the active node and roll out upgrades without downtime.

vault-mgmt talks to the Kubernetes API using your current kubeconfig context and reaches the Vault API through a port-forward to each pod, so Vault does not need to be exposed outside the cluster.

## Features

- **Show** the state of all pods: phase, image, and whether each one is initialized, sealed, active and ready.
- **Unseal** all sealed pods, with keys read from a Vault KV secret or printed by a local command.
- **Step down** the active pod.
- **Upgrade** the whole cluster without downtime: standby pods are upgraded first, then the active pod steps down and is upgraded last.
- **Exec** a command in the active, a standby or a sealed pod.
- **Wait** until the StatefulSet is ready.

## Requirements

- Vault or OpenBao deployed as a StatefulSet, e.g. with the official Helm chart. The pods are found by the selector of the StatefulSet, so several clusters can share a namespace.
- [Kubernetes service registration](https://developer.hashicorp.com/vault/docs/configuration/service-registration/kubernetes) enabled, so the pods carry the `vault-initialized`, `vault-sealed` and `vault-active` labels (`openbao-*` for OpenBao).
- Kubernetes 1.32 or newer.
- Permissions in the namespace to get, list and delete pods, to create `pods/exec` and `pods/portforward`, and to get, list and watch StatefulSets.

## Installation

Download a prebuilt binary for Linux or macOS (x86_64 and arm64) from the [releases page](https://github.com/nimbolus/vault-mgmt/releases), or build it from source:

```sh
cargo install --git https://github.com/nimbolus/vault-mgmt
```

Release binaries can update themselves with `vault-mgmt self-update`.

## Usage

```sh
# show the state of the cluster in namespace "vault"
vault-mgmt show

# OpenBao in another namespace
vault-mgmt --flavor openbao --namespace openbao --statefulset openbao show

# unseal all sealed pods, reading the keys from a KV secret
export VAULT_TOKEN=...
vault-mgmt unseal --keys-secret-uri https://vault.example.com/v1/secret/data/vault/unseal-keys

# unseal with keys printed by a local command, one key per line
vault-mgmt unseal --key-cmd "pass show vault/unseal-keys"

# step down the active pod
vault-mgmt step-down

# rolling upgrade after changing the image of the StatefulSet
vault-mgmt upgrade --key-cmd "pass show vault/unseal-keys"

# run a command in a standby pod
vault-mgmt exec --in standby -- vault status
```

Run `vault-mgmt help <command>` for all options of a command.

### Global options

| Option                | Default | Description                                    |
|-----------------------|---------|------------------------------------------------|
| `-n`, `--namespace`   | `vault` | Namespace Vault is running in                  |
| `-s`, `--statefulset` | `vault` | StatefulSet whose pods are managed             |
| `-f`, `--flavor`      | `vault` | `vault` or `openbao`                           |
| `-d`, `--domain`      | `vault` | Domain name used to verify the TLS certificate |
| `--no-tls`            |         | Talk plain HTTP to Vault                       |
| `-l`, `--log-level`   | `info`  | `error`, `warn`, `info`, `debug` or `trace`    |

### Tokens and unseal keys

Commands that need a Vault token (`unseal` with `--keys-secret-uri`, `step-down`, `upgrade`) take it from `--token` or the `VAULT_TOKEN` environment variable.

Unseal keys come from one of two sources:

- `--keys-secret-uri`: a KV secret whose field `keys` contains the keys separated by newlines.
- `--key-cmd`: a command run locally that prints one key per line.

### Upgrades

`upgrade` deletes each pod whose image tag differs from the one in the StatefulSet and waits for it to be recreated and unsealed. Standby pods go first. Then the active pod steps down and is upgraded once a standby has taken over.

- `--force-upgrade` rolls every pod, even if it is already up to date, for example to pick up rotated certificates.
- `--do-not-unseal` waits for the pods to be unsealed by something else, e.g. auto-unseal.

### Shell completion

```sh
vault-mgmt completion zsh > "${fpath[1]}/_vault-mgmt"
```

Supported shells are `bash`, `elvish`, `fish`, `powershell` and `zsh`.

## Development

Unit tests run without a cluster:

```sh
cargo test
```

The end-to-end tests install, upgrade and uninstall several Vault and OpenBao clusters with Helm in the current kubeconfig context. They need:

- the `helm` CLI,
- a namespace, `vault-mgmt-e2e` by default (override with `VAULT_MGMT_E2E_NAMESPACE`). `e2e-preparation.yaml` creates it together with a NetworkPolicy.

The pods use `emptyDir` storage and do not need persistent volumes. The tests cover clustering and active/standby transitions, not storage. Releases are uninstalled afterwards, except when a test fails, to leave them for debugging.

```sh
kubectl apply -f e2e-preparation.yaml
cargo test -- --ignored
```

## License

[MIT](LICENSE)
