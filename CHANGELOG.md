# [1.3.0](https://github.com/TopGunBuild/topgun/compare/v1.2.4...v1.3.0) (2023-06-03)


### Bug Fixes

* **client:** add createClient function ([047cc9a](https://github.com/TopGunBuild/topgun/commit/047cc9ae156efdf1d451dbb24380613d6ded2403))
* **client:** add option for max limit for LEX query ([882e438](https://github.com/TopGunBuild/topgun/commit/882e438cb33b168b4c3dfd3685a0b51dda9f93c3))
* **client:** add wait for client disconnect ([23a7b51](https://github.com/TopGunBuild/topgun/commit/23a7b51e9826a74a4697cb1341f7848a1163adb6))
* **client:** create LEX object in constructor ([b26dd36](https://github.com/TopGunBuild/topgun/commit/b26dd36ba877d88e489ab1bb8605d9e62029d301))
* **client:** get soul from optionsGet ([a722755](https://github.com/TopGunBuild/topgun/commit/a722755eb3d979c3ec03725ac72fdb578fa8149a))
* **client:** handle optionsGet property in link class ([a8c1436](https://github.com/TopGunBuild/topgun/commit/a8c1436cf5c78b576894306c9c1e2b20262afde0))
* **client:** make disconnect async ([0c3df5d](https://github.com/TopGunBuild/topgun/commit/0c3df5d49f15668c0202563176e564b8cab18c92))
* **client:** pass optionsGet to subscribe request ([ab2e002](https://github.com/TopGunBuild/topgun/commit/ab2e0027b4cbb3aa3aa63d0e54fec50417a2085e))
* upgrade topgun-socket 1.4.2 -> 1.4.3 ([a9c8034](https://github.com/TopGunBuild/topgun/commit/a9c8034dc6f1716c682b8ea8e2ebef6711391b1c))
* upgrade topgun-socket 1.4.4 -> 1.4.7 ([329bc58](https://github.com/TopGunBuild/topgun/commit/329bc584e6c1adaa79b1cf99e7b36ac1a1b846c1))


### Features

* **client:** add client method to disconnect ([25dbe99](https://github.com/TopGunBuild/topgun/commit/25dbe99e13aa9f2d22139d6c82e7c1ef8a5eb292))
* **client:** add passwordMaxLength option ([d78561b](https://github.com/TopGunBuild/topgun/commit/d78561b48d8219a59aaa7bc3813ae9d430467585))
* **client:** the ability to set peer options by an object ([0c53350](https://github.com/TopGunBuild/topgun/commit/0c53350dc9496f1b1e269eafddea685c5c46329c))
* **server:** add server methods to stop and check readiness ([0f7201f](https://github.com/TopGunBuild/topgun/commit/0f7201fbf2f865842fcc6706c6a75e99049b1dad))

## [1.2.4](https://github.com/TopGunBuild/topgun/compare/v1.2.3...v1.2.4) (2023-05-29)


### Bug Fixes

* upgrade topgun-socket 1.4.3 -> 1.4.4 ([2f1dd51](https://github.com/TopGunBuild/topgun/commit/2f1dd5104dbe4dd013e09c35d9c83e14c78446c2))

## [1.2.3](https://github.com/TopGunBuild/topgun/compare/v1.2.2...v1.2.3) (2023-05-29)


### Bug Fixes

* upgrade topgun-socket 1.4.2 -> 1.4.3 ([4d0c1dd](https://github.com/TopGunBuild/topgun/commit/4d0c1dd9b8c482e07616b0cd641c2a29fd1531d6))

## [1.2.2](https://github.com/TopGunBuild/topgun/compare/v1.2.1...v1.2.2) (2023-05-28)


### Bug Fixes

* fix subscribe data ([b13a324](https://github.com/TopGunBuild/topgun/commit/b13a324478dfe0195229920723fde2fb67b38330))

## [1.2.1](https://github.com/TopGunBuild/topgun/compare/v1.2.0...v1.2.1) (2023-05-28)


### Bug Fixes

* upgrade topgun-socket 1.3.3 -> 1.4.2 ([62bbc75](https://github.com/TopGunBuild/topgun/commit/62bbc75cec7e2f80484313dbc5c6ba494747261a))

# [1.2.0](https://github.com/TopGunBuild/topgun/compare/v1.1.2...v1.2.0) (2023-05-06)


### Bug Fixes

* **client:** fix value assignment to persistSession ([ad534b6](https://github.com/TopGunBuild/topgun/commit/ad534b60255912f6a85cbffc8ef08de1a6aa3f70))
* fix CRDT diff check ([042a49a](https://github.com/TopGunBuild/topgun/commit/042a49a74bbf4ca26987fff4502f3b5ba9516409))


### Features

* **client:** listen data in user space before authorization ([a353d6d](https://github.com/TopGunBuild/topgun/commit/a353d6d8f10a1be8082dd40d29ab31ab05f4a8df))

## [1.1.2](https://github.com/TopGunBuild/topgun/compare/v1.1.1...v1.1.2) (2023-05-03)


### Bug Fixes

* upgrade topgun-buffer 1.0.5 -> 1.0.6 ([0515c23](https://github.com/TopGunBuild/topgun/commit/0515c231c10417486c8c13c13435af0195556aa0))
* upgrade topgun-socket 1.3.2 -> 1.3.3 ([8d2528a](https://github.com/TopGunBuild/topgun/commit/8d2528a4507675b67e26ae093476f53c05bc269a))

## [1.1.1](https://github.com/TopGunBuild/topgun/compare/v1.1.0...v1.1.1) (2023-05-03)


### Bug Fixes

* add missing Buffer import ([5e308e9](https://github.com/TopGunBuild/topgun/commit/5e308e93c42d02f291120117664630c8f6cfcb45))
* **sea:** fix Buffer typings ([eab2f6a](https://github.com/TopGunBuild/topgun/commit/eab2f6aa6f38d31e1a3d9ab329e1095bd5de439c))
* upgrade topgun-buffer 1.0.3 -> 1.0.5 ([2602c7e](https://github.com/TopGunBuild/topgun/commit/2602c7e09c0a0776677d47b1cc6fc86e23fa5567))
* upgrade topgun-socket 1.3.1 -> 1.3.2 ([e70a35a](https://github.com/TopGunBuild/topgun/commit/e70a35a96377708b661c3a49fad5405f427d9754))

# [1.1.0](https://github.com/TopGunBuild/topgun/compare/v1.0.4...v1.1.0) (2023-05-01)


### Bug Fixes

* bump topgun-socket version, update server, add example server ([0e944d2](https://github.com/TopGunBuild/topgun/commit/0e944d2d2025ca6fea293f55f7fd3256bc38d606))
* use topgun-typed instead of ajv ([1bd3aa0](https://github.com/TopGunBuild/topgun/commit/1bd3aa0fb78c68b84710df86219a3d8980c5a046))


### Features

* add port to TGServerOptions ([cf5c3ce](https://github.com/TopGunBuild/topgun/commit/cf5c3ce0a473b424d27dbda01ec3e8b3891248e6))

## [1.0.4](https://github.com/TopGunBuild/topgun/compare/v1.0.3...v1.0.4) (2023-04-18)


### Bug Fixes

* bump topgun-socket 1.2.2 -> 1.2.3 ([95c50f6](https://github.com/TopGunBuild/topgun/commit/95c50f60a0723f389bf3e4c69c69833d005f56de))
* export `SEA` methods from client ([4c4d3fb](https://github.com/TopGunBuild/topgun/commit/4c4d3fbe37f9f50b270ff779620694446da97202))
* set `splitting` to false ([76e9d46](https://github.com/TopGunBuild/topgun/commit/76e9d46ae7e4fd5255b19d9f3449acd4e81587aa))

## [1.0.3](https://github.com/TopGunBuild/topgun/compare/v1.0.2...v1.0.3) (2023-04-18)


### Bug Fixes

* add topgun-buffer, upgrade deps, add examples folder ([2f00aa4](https://github.com/TopGunBuild/topgun/commit/2f00aa4bf7af5a9bdc21ad421501b495778e55ac))
* bump topgun-socket 1.1.10 -> 1.1.11 ([c6eec0f](https://github.com/TopGunBuild/topgun/commit/c6eec0fa4f20ab1acdbf731c2ab5a8e6eeb523d7))
* update "exports" in package.json ([80eaff7](https://github.com/TopGunBuild/topgun/commit/80eaff74886bd56f4545e75acf223301e0efa5db))
* upgrade deps ([09daf8d](https://github.com/TopGunBuild/topgun/commit/09daf8d20e649c55768910e39e6508353a47f8ac))

## [1.0.2](https://github.com/TopGunBuild/topgun/compare/v1.0.1...v1.0.2) (2023-04-13)


### Bug Fixes

* **client:** return graph key in put callback ([f68a810](https://github.com/TopGunBuild/topgun/commit/f68a8109c4f97443d686031581ea8f71d12eea9e))

## [1.0.1](https://github.com/TopGunBuild/topgun/compare/v1.0.0...v1.0.1) (2023-04-13)


### Bug Fixes

* using topgun-webcrypto instead of @peculiar/webcrypto for isomorphic support ([b2444e3](https://github.com/TopGunBuild/topgun/commit/b2444e38e4df8f6e7e1c1de8aa89962991976bce))

# 1.0.0 (2023-04-13)


### Bug Fixes

* **client:** fix options type ([dd93706](https://github.com/TopGunBuild/topgun/commit/dd93706f3bffb6a613a686609866050ac12daf6e))


### Features

* add simple-git-hooks with "pre-commit" hook ([5214f25](https://github.com/TopGunBuild/topgun/commit/5214f259bff2c4f29efb08b8c66b361a48f4b1b5))
