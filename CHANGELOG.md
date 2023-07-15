## [1.5.3](https://github.com/TopGunBuild/topgun/compare/v1.5.2...v1.5.3) (2023-07-15)


### Bug Fixes

* add type for user graph ([bd1740d](https://github.com/TopGunBuild/topgun/commit/bd1740d346aa234e5b61a47eacbe67e4a22dd904))
* **client:** add `state` property to graph ([3f10f71](https://github.com/TopGunBuild/topgun/commit/3f10f7100c692efe4c5401f7b1454541e05c492a))
* **client:** update user sign graph ([ca48f07](https://github.com/TopGunBuild/topgun/commit/ca48f078bb57c44e9adef90642d16a55bfbf836b))
* improve connectors ([07cdd3b](https://github.com/TopGunBuild/topgun/commit/07cdd3b9173cfe1342acc309d1e72cb717850554))
* improve direct query reply ([170a93d](https://github.com/TopGunBuild/topgun/commit/170a93d9be145a673eb09c6ee7eb0e15f5685961))
* return user pairs after authorization ([1ed920a](https://github.com/TopGunBuild/topgun/commit/1ed920a61684ca59b6006d09217ead408a2048aa))
* **sea:** export Pair interfaces ([bb640dc](https://github.com/TopGunBuild/topgun/commit/bb640dc8d1a175f215ecb16716296c8b768fc6db))
* update export packages ([7ed8c36](https://github.com/TopGunBuild/topgun/commit/7ed8c3648ae84b1fef262ee8fda11798a5fc7fd0))
* update typings ([83060a1](https://github.com/TopGunBuild/topgun/commit/83060a177e8f40d8c9b2571e91003b9637680ad0))

## [1.5.2](https://github.com/TopGunBuild/topgun/compare/v1.5.1...v1.5.2) (2023-07-10)


### Bug Fixes

* add stream and sea packages for export ([e70a0bb](https://github.com/TopGunBuild/topgun/commit/e70a0bbc157186a1d2e0dc6109b28951d8ffb2bb))
* **client:** check username in use before create user ([2a2239e](https://github.com/TopGunBuild/topgun/commit/2a2239e967ea201f5658b5b10352f04ed7d35a57))
* upgrade exports ([3d3af8d](https://github.com/TopGunBuild/topgun/commit/3d3af8dff60c60b8abd1bfa36ae82202ec5bbdad))

## [1.5.1](https://github.com/TopGunBuild/topgun/compare/v1.5.0...v1.5.1) (2023-07-08)


### Bug Fixes

* **client:** improve `leave()` method ([58b43a9](https://github.com/TopGunBuild/topgun/commit/58b43a93951a2743c82e8a9cdb9b069a05bf18ac))
* **client:** update options, add `waitForConnect()` method ([a5c42c0](https://github.com/TopGunBuild/topgun/commit/a5c42c04fe13fe7fc1154f4f023152ce77f84739))
* fix listen once ([4acc73c](https://github.com/TopGunBuild/topgun/commit/4acc73c5eeb804df68806f0c4d153adadc2f30ed))
* remove unused export ([198ebfb](https://github.com/TopGunBuild/topgun/commit/198ebfb5ecc2e72552b8eb8cb792d406e945394d))
* wait for socket connector auth ([6eacb9c](https://github.com/TopGunBuild/topgun/commit/6eacb9cf132b5001d4b1ca9edcd9c17b69138cfc))

# [1.5.0](https://github.com/TopGunBuild/topgun/compare/v1.4.0...v1.5.0) (2023-07-02)


### Bug Fixes

* **client:** add `on` method to listen for system events ([b664cea](https://github.com/TopGunBuild/topgun/commit/b664cea42f2dd1928bd2cbf73c0965e6dd84c0ea))
* **client:** fix link back ([1114d9e](https://github.com/TopGunBuild/topgun/commit/1114d9ecfe545411867b9b0679fb4e6a5a8cb468))
* **client:** immutable link after `set()` ([c7c8796](https://github.com/TopGunBuild/topgun/commit/c7c8796e02cb6f8ec8809ea6cf660cda3a900b3e))
* **client:** rename method userPubExpected -> waitForAuth ([5a35600](https://github.com/TopGunBuild/topgun/commit/5a35600cb48c9a29baa82cfcf654c8e81598eb83))
* **client:** update link after authorization ([e103a76](https://github.com/TopGunBuild/topgun/commit/e103a76b4f1dce11f977ec4c2a0ab60458b435f4))
* **client:** upgrade topgun-async-stream-emitter 1.1.0 -> 1.1.1, add `stream` method ([6e21f18](https://github.com/TopGunBuild/topgun/commit/6e21f18f6fceee2e0486b5a5c544664a4910b17c))
* **types:** rename SystemEvent -> TGSystemEvent ([b02bd22](https://github.com/TopGunBuild/topgun/commit/b02bd22bab8b615302af5d3099b7d0d67018b47a))
* upgrade topgun-typed 1.2.0 -> 1.3.0, throw error when path not valid ([cb4c050](https://github.com/TopGunBuild/topgun/commit/cb4c050bd828ef751adc9e48bc01952ccf04a2fb))


### Features

* **client:** implement async iterable streams ([f44e28d](https://github.com/TopGunBuild/topgun/commit/f44e28dab2155fdf28d6b352b7de770a2f03cd8f))

# [1.4.0](https://github.com/TopGunBuild/topgun/compare/v1.3.0...v1.4.0) (2023-06-26)


### Bug Fixes

* add `promise` method to lex link ([329720e](https://github.com/TopGunBuild/topgun/commit/329720e61b199f912bb04d682cfdaae025509853))
* add path validation ([e47718e](https://github.com/TopGunBuild/topgun/commit/e47718ec59dfc374afad34a7e829e3b37a74f2fc))
* add topgun-textencoder, remove shims ([c0f5375](https://github.com/TopGunBuild/topgun/commit/c0f537515826ef3b497a81eec47a311af02a19d0))
* authenticate identity with a reference soul ([e26bcb3](https://github.com/TopGunBuild/topgun/commit/e26bcb307e5208f19a59b6dc7f44387dda8ac600))
* **client:** collections query fixed ([f7afb07](https://github.com/TopGunBuild/topgun/commit/f7afb07b67482e372dbb36c1bda5cc2663634697))
* **client:** generate soul in method `set` ([ad5be79](https://github.com/TopGunBuild/topgun/commit/ad5be79f9c310a609ffda2585e813bffcecf9b67))
* **client:** init get options in constructor ([ef2ddb0](https://github.com/TopGunBuild/topgun/commit/ef2ddb028accc28d24d531a0f629e4fef791bb53))
* **client:** move `set` method to lex-link ([4e29ca4](https://github.com/TopGunBuild/topgun/commit/4e29ca455f491780ef32d555535679ac177454b1))
* **client:** query multiple nodes ([c62fc72](https://github.com/TopGunBuild/topgun/commit/c62fc72a98ef49eb0b1b55ddfe29b3b57d1e0505))
* **client:** remove `persistSession` from options ([6f40894](https://github.com/TopGunBuild/topgun/commit/6f40894959d6f9bad20a97a45a99e76a102110eb))
* **client:** remove credentials when user leave ([07525ff](https://github.com/TopGunBuild/topgun/commit/07525ffba2a8da2e0d0315a689d9937f3aed5acc))
* **client:** remove default timeout in promise method ([039221a](https://github.com/TopGunBuild/topgun/commit/039221a3d1e7452e7479ccaea5a42547ec67e059))
* **client:** remove unused code ([279ef61](https://github.com/TopGunBuild/topgun/commit/279ef6168efbfb366f0af7044bc35f217dd5d37d))
* **client:** renamed local storage options ([c9aaaf0](https://github.com/TopGunBuild/topgun/commit/c9aaaf01ca52dd998ea8c99d97cd0c9fb0c4682d))
* **client:** resilve promise immediately if there are no active connectors ([7c07586](https://github.com/TopGunBuild/topgun/commit/7c075862ccafe4f7d98d1f1d9a23ad987bb86986))
* **client:** return soul from last value ([11a3017](https://github.com/TopGunBuild/topgun/commit/11a3017ddc5561445be9700b6530f65c15b61693))
* **client:** set default session storage ([df26d74](https://github.com/TopGunBuild/topgun/commit/df26d74ddc3229a54c88fcd129bad143e4c45e99))
* create memory adapter based on the memory storage ([e33e3b5](https://github.com/TopGunBuild/topgun/commit/e33e3b516d5ee8f84dfb4f314b65b23cf30a9143))
* fixed method `once` when requesting collections ([5278d4f](https://github.com/TopGunBuild/topgun/commit/5278d4f07ee0d3a8d6b159c6255f258e7f5ad7b5))
* flatten graph data ([8f497e9](https://github.com/TopGunBuild/topgun/commit/8f497e9e9413afdfd7b88e4f51b50784f6b9a252))
* include soul to prefix query ([16a41dc](https://github.com/TopGunBuild/topgun/commit/16a41dc234f113844428d06952c8ec06a2fb5dd7))
* **indexeddb:** get keys instead of values ([9a3e942](https://github.com/TopGunBuild/topgun/commit/9a3e942560f5c3393bea0f979721b4b711fa4321))
* request nodes from an adapter using get options ([3c5ad9d](https://github.com/TopGunBuild/topgun/commit/3c5ad9dc465fbb5d152539dd6ea8d34a97c36d84))
* update link soul ([547d948](https://github.com/TopGunBuild/topgun/commit/547d948fdcb373adca54886b489de9e074393705))


### Features

* add options for max key/value size ([b200d25](https://github.com/TopGunBuild/topgun/commit/b200d2582a65f46422984753ebf5e72a8d2b5311))

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
