# vault-mgmt

## Requirements
+ Vault is running in Kubernetes.
+ [Service Registration](https://developer.hashicorp.com/vault/docs/configuration/service-registration/kubernetes) is configured

## Testing
Unit tests can be run normally by cargo: `cargo test`.

Ent-to-end tests require a Kubernetes cluster and will install, upgrade and uninstall (except on failure) several deployments of a Vault cluster in the current `kubecontext` (namespace is set by environment variable `VAULT_MGMT_E2E_NAMESPACE`, defaulting to `vault-mgmt-e2e`).
The Pods are using `emptyDir` as storage and should not consume a PV.
The storage is not part of the tests, only the clustering and active/standby transitions.
You can run those tests by calling `cargo test --ignored` with a working `kubeconfig` and existing namespace.