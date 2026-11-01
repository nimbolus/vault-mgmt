# Changelog

### [v0.2.1](https://github.com/nimbolus/vault-mgmt/compare/v0.2.0...v0.2.1) (2026-11-01)

## [v0.2.0](https://github.com/nimbolus/vault-mgmt/compare/v0.1.8...v0.2.0) (2026-11-01)

### Features

* add reload command to pick up renewed certificates ([611684e](https://github.com/nimbolus/vault-mgmt/commit/611684e2da8ea7495ecccac4fdfba7e692928b53))
* retry kubernetes api requests on transient errors ([80198f7](https://github.com/nimbolus/vault-mgmt/commit/80198f7b76d56e9c69133c5f54a10ab907a65f46))

### Fixes

* time out vault api requests and polls over the port-forward ([0fccf2c](https://github.com/nimbolus/vault-mgmt/commit/0fccf2c01cff7a7cc0393d98d8c941dc319c1726))
* select pods by the statefulset's selector ([30fbfb5](https://github.com/nimbolus/vault-mgmt/commit/30fbfb5843fb7b78fa97b9ef9940ea1c4212a5d4))
* take the exec status before joining the process ([81cec88](https://github.com/nimbolus/vault-mgmt/commit/81cec88f1a5002e678221d271a212ca70553d488))

### [v0.1.8](https://github.com/nimbolus/vault-mgmt/compare/v0.1.7...v0.1.8) (2026-08-24)

#### Fixes

* support OpenBao seal-status response missing build_date ([fa4e500](https://github.com/nimbolus/vault-mgmt/commit/fa4e5007c1dd759f082d6f68cad3f3439d80e20c))

### [v0.1.7](https://github.com/nimbolus/vault-mgmt/compare/v0.1.6...v0.1.7) (2025-08-24)

#### Features

* add support for openbao flavor ([e879464](https://github.com/nimbolus/vault-mgmt/commit/e8794646c6cfb5a920c3ad32be932ef261b72a9a))

### [v0.1.6](https://github.com/nimbolus/vault-mgmt/compare/v0.1.5...v0.1.6) (2025-04-24)

### [v0.1.5](https://github.com/nimbolus/vault-mgmt/compare/v0.1.4...v0.1.5) (2025-02-15)

### [v0.1.4](https://github.com/nimbolus/vault-mgmt/compare/v0.1.3...v0.1.4) (2024-08-12)

#### Fixes

* **e2e:** wait for deletion of pod instead of non-running state ([53b5d24](https://github.com/nimbolus/vault-mgmt/commit/53b5d24df3df3744590b40a43d0898e13a3d001a))

### [v0.1.3](https://github.com/nimbolus/vault-mgmt/compare/v0.1.2...v0.1.3) (2023-09-29)

#### Features

* retry port-forwarding request to accomodate starting process ([02a726e](https://github.com/nimbolus/vault-mgmt/commit/02a726e8f4a02894d6819b6eb19fa63bf610ad5b))

#### Fixes

* wait for pod to set its status labels ([f9eb3fe](https://github.com/nimbolus/vault-mgmt/commit/f9eb3fe3a28a519a7361e74adee6f61d30445752))
* external unseal in cli ([3d185dc](https://github.com/nimbolus/vault-mgmt/commit/3d185dce1f72293cca53b97e89672d04ff6fe4d0))
* spawn blocking task for self-update ([290287e](https://github.com/nimbolus/vault-mgmt/commit/290287ea12a1227bd1dcfe0ffaf97c1b2ee67521))

### v0.1.2 (2023-06-11)

#### Features

* release binaries, add self-update command and rename cli back to vault-mgmt ([d52de41](https://github.com/nimbolus/vault-mgmt/commit/d52de41e35f8b560bde9ea82414ea4f426a5f926))
* cleanup cli args and add autocompletion generation ([4a90961](https://github.com/nimbolus/vault-mgmt/commit/4a90961b0a9209714081db42a9ab42cbae457041))
* retrieve unseal keys from vault secret ([cd70f3b](https://github.com/nimbolus/vault-mgmt/commit/cd70f3b82ffde7ce14d1b512ae0f180fce5778d7))
* add option to force upgrade of pods ([ecb3f91](https://github.com/nimbolus/vault-mgmt/commit/ecb3f91bb5216fb98a7469e61ff87cdcd8b9062e))
* add option for external unseal ([f479576](https://github.com/nimbolus/vault-mgmt/commit/f479576cf71cb70d0893660700eaaf2eb2673bc6))
* improve and add tests ([09630a0](https://github.com/nimbolus/vault-mgmt/commit/09630a0eeaff639c30c0f69395c5dbc6756707ce))
* use Swatinem/rust-cache in ci ([2e3b4f6](https://github.com/nimbolus/vault-mgmt/commit/2e3b4f69a475ee8fd6505f814a8a8bc601bddb35))
* add build with artifact to ci ([db908e7](https://github.com/nimbolus/vault-mgmt/commit/db908e7b7b173eaa249891b47b7b5cfe9a58075d))
* run tests in ci (no tests yet) ([f801ac1](https://github.com/nimbolus/vault-mgmt/commit/f801ac16ee702d2dbf08639fc04b2410ba73636c))
* implement vault upgrade mechanism ([792516d](https://github.com/nimbolus/vault-mgmt/commit/792516df8758459432dfc800d1dd5dcfe4f830e8))

#### Fixes

* use tokio::io instead std::io for exec command ([26890bb](https://github.com/nimbolus/vault-mgmt/commit/26890bb8c0784ac983626f3decdced7d6c0aedc3))
* ci release path vault-mgmt-cli ([402a64e](https://github.com/nimbolus/vault-mgmt/commit/402a64eaa92495be448116cbddc5a72f21c80bc5))
* remove incorrect waiting condition ([cc67dd3](https://github.com/nimbolus/vault-mgmt/commit/cc67dd3efb668229d6ee7abe5cde2fc7621e7038))
