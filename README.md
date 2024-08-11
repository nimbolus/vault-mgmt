# vault-mgmt

## Requirements
+ Vault is running in Kubernetes.
+ [Service Registration](https://developer.hashicorp.com/vault/docs/configuration/service-registration/kubernetes) is configured

## Features
+ Unseal a Vault Pod.
  + Either supply a command that returns the unseal keys
  + or let the program retrieve the keys from a Vault secret.
+ Step-down the active Pod.
+ Upgrade a single Pod.
+ Upgrade the full cluster without downtime.

## Testing
Unit tests can be run normally by cargo: `cargo test`.

End-to-end tests require a Kubernetes cluster and will install, upgrade and uninstall (except on failure) several deployments of a Vault cluster in the current `kubecontext` (namespace is set by environment variable `VAULT_MGMT_E2E_NAMESPACE`, defaulting to `vault-mgmt-e2e`). You can create the Namespace and NetworkPolicy from `e2e-preparation.yaml`.
The Pods are using `emptyDir` as storage and should not consume a PV.
The storage is not part of the tests, only the clustering and active/standby transitions.
You can run those tests by calling `cargo test --ignored` with a working `kubeconfig` and existing namespace.
