# vault-mgmt

Manage [Vault](https://www.vaultproject.io/) and [OpenBao](https://openbao.org/) clusters running in Kubernetes: inspect their state, unseal pods, step down the active node and roll out upgrades without downtime.

vault-mgmt talks to the Kubernetes API using your current kubeconfig context and reaches the Vault API through a port-forward to each pod, so Vault does not need to be exposed outside the cluster.

## Features

- **Show** the state of all pods: phase, image, and whether each one is initialized, sealed, active and ready.
- **Unseal** all sealed pods, with keys read from a Vault KV secret or printed by a local command.
- **Step down** the active pod.
- **Reload** the configuration of all pods, e.g. to pick up renewed TLS certificates.
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

# reload all pods, e.g. after their TLS certificate was renewed
vault-mgmt reload

# reload only the standby pods
vault-mgmt reload --in standby

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

- `--force-upgrade` rolls every pod, even if it is already up to date, for example after changing its configuration. Renewed certificates do not need a restart, see [Certificate renewal](#certificate-renewal).
- `--do-not-unseal` waits for the pods to be unsealed by something else, e.g. auto-unseal.

### Certificate renewal

Vault reads its TLS certificate only at startup and on SIGHUP. When the certificate is renewed, e.g. by cert-manager, the pods keep serving the old one until it expires. `reload` sends SIGHUP to the `vault server` process (`bao server` for OpenBao) in every pod, so the renewed certificate is used without a restart. `--in active`, `--in standby` or `--in sealed` limits it to the pods in that state. If no pod matches, `reload` fails, unless `--ignore-not-found` is given, e.g. for `--in sealed` in a scheduled job. Sending SIGHUP to pid 1 of the container is not enough with the Helm chart, as pid 1 is a shell that does not forward it.

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

### Releasing

Pushing a `v*` tag runs the release workflow: it runs the test suite, checks that `CHANGELOG.md` is up to date, creates a draft release with the changelog section of the version as notes, builds the binaries and publishes the release.

The changelog check regenerates `CHANGELOG.md` from the tag and compares it with the committed one. The version heading takes its date and compare link from the annotated tag. `--tag-unreleased` renders the unreleased commits as if `HEAD` was already tagged, so the changelog is generated before the tag and the tag goes on the release commit:

```sh
v=0.2.0
convco changelog --ignore-prereleases --unreleased "$v" --tag-unreleased --output CHANGELOG.md
git add CHANGELOG.md && git commit -m "chore: release v$v"
git tag -a "v$v" -m "v$v"
git push origin main "v$v"
```

- `--tag-unreleased` is not in a convco release yet, it needs a build of the `changelog-unreleased-tag` branch of [tobikris/convco](https://github.com/tobikris/convco). CI does not need it, it generates the changelog from the pushed tag.
- `--unreleased` takes the version without the `v` prefix.
- The tag has to be annotated and created the same day, a lightweight tag gets the commit date.
- `chore:` commits are not listed in the changelog, so the release commit does not change it.

Release candidates (tags with a `-`, like `v0.2.0-rc.1`) are published as prereleases. They are not added to `CHANGELOG.md` and need no release commit, the tag is pushed directly. The changelog check is skipped for them, their release notes cover the commits since the previous tag.

## License

[MIT](LICENSE)
