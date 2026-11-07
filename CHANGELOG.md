# Changelog

## [0.10.0](https://github.com/cedricziel/folio/compare/v0.9.1...v0.10.0) (2026-11-07)


### Features

* **core:** add modified_since and CompleteIdSet to PaperlessReader ([d3b5b1c](https://github.com/cedricziel/folio/commit/d3b5b1c6be312d29390b935d91e98768f6cdd203))
* **core:** add the PaperlessReader port ([29aa8cd](https://github.com/cedricziel/folio/commit/29aa8cd0869c944950ba9c447d52832f788ffb8c))
* **infra:** add [import] config section for allowed roots and file size limit ([1cde079](https://github.com/cedricziel/folio/commit/1cde079e307f08ac7b0a025ab6a41e091654e7fe))
* **infra:** confine import directories to the allowed roots ([5f447de](https://github.com/cedricziel/folio/commit/5f447deed8f734b7dcb6cb21f0bc0ce35e030433))
* **infra:** import config section and Paperless migration guide ([#332](https://github.com/cedricziel/folio/issues/332)) ([4f2c75c](https://github.com/cedricziel/folio/commit/4f2c75cc60eb96e408977677cafa9d60f51d692a))
* **infra:** Paperless export reader and path guards ([#331](https://github.com/cedricziel/folio/issues/331)) ([ba3c65f](https://github.com/cedricziel/folio/commit/ba3c65f749357ddef2035e3b1d911a2cdf77ed93))
* **infra:** refuse unsafe paths to Paperless originals ([6c405cc](https://github.com/cedricziel/folio/commit/6c405cc9747d88dad07761a65fb44802fce0ee1e))
* **infra:** stream Paperless-ngx exports through PaperlessReader ([a0ff42a](https://github.com/cedricziel/folio/commit/a0ff42a6f40e68cc36d0c9ff8c9d3eec250f00de))


### Bug Fixes

* **apple:** close a scan review only from its own completion ([#321](https://github.com/cedricziel/folio/issues/321)) ([a5d9034](https://github.com/cedricziel/folio/commit/a5d90340e29f62c6436d56b9ad68fe330a5a868b)), closes [#166](https://github.com/cedricziel/folio/issues/166)
* **core:** keep CompleteIdSet from being built empty by default ([b789e8a](https://github.com/cedricziel/folio/commit/b789e8abe661d62f8754bc14c8c3beceb35c40cc))
* **core:** keep CompleteIdSet from being built empty by default ([#334](https://github.com/cedricziel/folio/issues/334)) ([707ea45](https://github.com/cedricziel/folio/commit/707ea45a1ee4200cd5cc460acd14cad974bd85f0))
* **infra:** export unsigned span arguments as integers ([9ff8628](https://github.com/cedricziel/folio/commit/9ff8628a73af4ce87e88a23d53e1e93f93b7e206))
* **infra:** fail single Paperless documents instead of whole scans ([0cab06b](https://github.com/cedricziel/folio/commit/0cab06b3ac694366c0c68c7523be55a684bdaef3))
* **infra:** open import directories while checking them against the roots ([ca48472](https://github.com/cedricziel/folio/commit/ca484724c477a48f5c24a3bfaf153a7876fe4854))
* **infra:** open Paperless originals without following any symlink ([a757928](https://github.com/cedricziel/folio/commit/a75792805b29f13b50280aa98c7d6542642cbced))
* **infra:** read Paperless exports through one directory handle ([d5fd804](https://github.com/cedricziel/folio/commit/d5fd8047f887dc49815aad67f82a25676776dd5a))
* **infra:** refuse originals swapped between the path check and the open ([c2aefe2](https://github.com/cedricziel/folio/commit/c2aefe25ca6c328b511e3ef9c989bc1c8ff04a17))
* **telemetry:** export the HTTP status code as an integer ([d8c8a1e](https://github.com/cedricziel/folio/commit/d8c8a1e794dc50d62c7f8edeba4d89604cf80d97))
* **telemetry:** export unsigned span attributes as integers ([#328](https://github.com/cedricziel/folio/issues/328)) ([d588efa](https://github.com/cedricziel/folio/commit/d588efa5126b9c0ac1958cf12fc9f80d83aa4739))

## [0.9.1](https://github.com/cedricziel/folio/compare/v0.9.0...v0.9.1) (2026-11-07)


### Bug Fixes

* **apple:** check download status before reading the error body ([c45feeb](https://github.com/cedricziel/folio/commit/c45feeb9225eda0e32bdfec90741b901d5da6753)), closes [#156](https://github.com/cedricziel/folio/issues/156)
* **apple:** check download status before reading the error body ([#322](https://github.com/cedricziel/folio/issues/322)) ([8aac098](https://github.com/cedricziel/folio/commit/8aac0989f2d3f048296b624b4a322849e2178b90)), closes [#156](https://github.com/cedricziel/folio/issues/156)
* **apple:** check the server again before a refresh changes credentials ([8724603](https://github.com/cedricziel/folio/commit/87246034e275f87cce8f032d62ddab91c714b1bb))
* **apple:** keep credentials bound to the server that issued them ([#305](https://github.com/cedricziel/folio/issues/305)) ([e53c38b](https://github.com/cedricziel/folio/commit/e53c38bf0a0ca1716fb66cb68c5f73a66dea11a5))
* **apple:** keep File Provider sibling names unique after sanitizing ([d15a22f](https://github.com/cedricziel/folio/commit/d15a22f2e72cc6b90811a26a5343cd1383e6cc99)), closes [#174](https://github.com/cedricziel/folio/issues/174)
* **apple:** keep File Provider sibling names unique after sanitizing ([#324](https://github.com/cedricziel/folio/issues/324)) ([77208d6](https://github.com/cedricziel/folio/commit/77208d62f1bb20f98de54dc20b91b205a1e354bc)), closes [#174](https://github.com/cedricziel/folio/issues/174)
* **apple:** name failed operation spans with error.type and status ([#301](https://github.com/cedricziel/folio/issues/301)) ([02a734b](https://github.com/cedricziel/folio/commit/02a734b5b51151fc2ae8b83ca17b6a8675e7dd61)), closes [#300](https://github.com/cedricziel/folio/issues/300)
* **auth:** invalidate pending second-factor logins after a password change ([#304](https://github.com/cedricziel/folio/issues/304)) ([51f560b](https://github.com/cedricziel/folio/commit/51f560bd6b4a35f7aaa789c8672d3828dbc2067a)), closes [#164](https://github.com/cedricziel/folio/issues/164)
* **web:** ask before moving a source or workflow to the trash ([0ecb1c7](https://github.com/cedricziel/folio/commit/0ecb1c739310729d36aa989fb7ac883213d39d25))
* **web:** ask before moving a source or workflow to the trash ([#326](https://github.com/cedricziel/folio/issues/326)) ([17b2950](https://github.com/cedricziel/folio/commit/17b2950ab48812af0edd409df58279655480c850)), closes [#130](https://github.com/cedricziel/folio/issues/130)
* **web:** force OCR when reading a document again ([090afa7](https://github.com/cedricziel/folio/commit/090afa79a03032253750efda9b094192336079ac)), closes [#160](https://github.com/cedricziel/folio/issues/160)
* **web:** send the unsaved edits when testing a workflow ([acfedb5](https://github.com/cedricziel/folio/commit/acfedb55f7f6dd808f07d3bdf5f7f0030d63919a))
* **web:** send the unsaved edits when testing a workflow ([#323](https://github.com/cedricziel/folio/issues/323)) ([352c705](https://github.com/cedricziel/folio/commit/352c7054f1bb1af33d3caeb09247e04f3016b9d9)), closes [#127](https://github.com/cedricziel/folio/issues/127)
* **worker:** rerun OCR for forced processing jobs ([23c883d](https://github.com/cedricziel/folio/commit/23c883db5847ebf1ca7bd1943afb6d1b8804d26b)), closes [#160](https://github.com/cedricziel/folio/issues/160)
* **worker:** rerun OCR when reading a document again ([#325](https://github.com/cedricziel/folio/issues/325)) ([7bf3cc0](https://github.com/cedricziel/folio/commit/7bf3cc02da09a307e97bd90344708fed5697d82a)), closes [#160](https://github.com/cedricziel/folio/issues/160)

## [0.9.0](https://github.com/cedricziel/folio/compare/v0.8.0...v0.9.0) (2026-11-03)


### Features

* **api:** expose authorized document entry CRUD ([839db54](https://github.com/cedricziel/folio/commit/839db54f0da86232e6549449e3f8949aa73a476f))
* **api:** expose authorized document entry CRUD ([#283](https://github.com/cedricziel/folio/issues/283)) ([2618a1a](https://github.com/cedricziel/folio/commit/2618a1a60adef1065b76afdc61a639f713525d7c))
* **api:** expose vault-scoped saved-view management ([8415973](https://github.com/cedricziel/folio/commit/84159731325f33666919900ca622a346b3da41e8))
* **api:** expose vault-scoped saved-view management ([#278](https://github.com/cedricziel/folio/issues/278)) ([1658afd](https://github.com/cedricziel/folio/commit/1658afd3e4a25bb206f815d724017c81c35cbecd))
* **api:** list source outcomes over JSON and the web ([473bc42](https://github.com/cedricziel/folio/commit/473bc4216d0b82b381015a7211e021a840906f6b))
* **api:** set review decisions and filter by review state ([e26d96b](https://github.com/cedricziel/folio/commit/e26d96bacc909c96fa97f8ea460fe93a2589e9c5))
* **apple:** add document notes and comments ([39477b6](https://github.com/cedricziel/folio/commit/39477b602974ad1ee4bd6f570569762c6d18a743))
* **apple:** add document notes and comments ([#292](https://github.com/cedricziel/folio/issues/292)) ([35cff77](https://github.com/cedricziel/folio/commit/35cff7739ae046d0e656802ac627f9781a965564))
* **apple:** add review controls and review and date filters ([91701e7](https://github.com/cedricziel/folio/commit/91701e7df519168d5379e6a3abd78e0de105ebd6))
* **apple:** manage classifications with trash and restore ([26f5dd4](https://github.com/cedricziel/folio/commit/26f5dd4117ac9efd128581699cb73c6d9d09269d))
* **apple:** manage classifications with trash and restore ([#315](https://github.com/cedricziel/folio/issues/315)) ([bb16198](https://github.com/cedricziel/folio/commit/bb161981230d2e7b793a986b2a395ac6830e386d))
* **apple:** save and restore document views ([5f201f9](https://github.com/cedricziel/folio/commit/5f201f912674493bdccff60e2c4fba700be85c3d))
* **apple:** save and restore document views ([#297](https://github.com/cedricziel/folio/issues/297)) ([b81ca98](https://github.com/cedricziel/folio/commit/b81ca98fc29c5d39d7b8ee9dbbd6b4634a80e457))
* **apple:** show source outcomes and activity ([1f47e65](https://github.com/cedricziel/folio/commit/1f47e650870117bd033eaec09782ad4f4ce91963))
* **apple:** show source outcomes and activity ([#316](https://github.com/cedricziel/folio/issues/316)) ([1ef0a92](https://github.com/cedricziel/folio/commit/1ef0a9205aaf4fade25f678274dbad73486b81e6))
* **core:** build processing stage domain events ([b00788e](https://github.com/cedricziel/folio/commit/b00788ed70a7a66b324d931bd724ba78d2a430c4))
* **core:** capture trusted lineage in domain events ([#266](https://github.com/cedricziel/folio/issues/266)) ([b605c80](https://github.com/cedricziel/folio/commit/b605c808d684d777482e68c8b6b05cfe344fc28e))
* **core:** define document note and comment contracts ([#257](https://github.com/cedricziel/folio/issues/257)) ([b01273d](https://github.com/cedricziel/folio/commit/b01273d52410c8e9cc16659ad450177b9b2e7e45))
* **core:** define the metadata value repository port ([c16c8d8](https://github.com/cedricziel/folio/commit/c16c8d8385c49f033749b2d158da8849a9bb52d7))
* **core:** define vault-owned metadata fields and options ([#267](https://github.com/cedricziel/folio/issues/267)) ([c5d9a72](https://github.com/cedricziel/folio/commit/c5d9a72621709a4be87e243f9953b5fd67d56144))
* **core:** model processing attempt outcomes ([f010b45](https://github.com/cedricziel/folio/commit/f010b45140b2bbb87b22953f60791185b35e1434))
* **core:** model processing attempt outcomes ([#270](https://github.com/cedricziel/folio/issues/270)) ([2353a4d](https://github.com/cedricziel/folio/commit/2353a4d73fa2a0785f525259f7a5dff41b312cd4))
* **db:** add typed metadata value storage ([8ff28f4](https://github.com/cedricziel/folio/commit/8ff28f41b1bc9dae04c1d0d11ae8081ff92e5d93))
* **db:** persist typed document metadata values ([#310](https://github.com/cedricziel/folio/issues/310)) ([e107015](https://github.com/cedricziel/folio/commit/e1070156bcedb7cd28ac59921f957ee78849d269))
* **db:** persist typed metadata values atomically ([9902691](https://github.com/cedricziel/folio/commit/9902691353841891d080176b733ebc6c1f07d19c))
* **documents:** derive inbox status from explicit review decisions ([8480ce9](https://github.com/cedricziel/folio/commit/8480ce91c0689428f0d3d3af5fa5dc8cff38443b))
* **documents:** edit calendar dates in web and Apple clients ([91ee12f](https://github.com/cedricziel/folio/commit/91ee12f5efe4444088827731d8f0018eb54e48eb))
* **documents:** edit calendar dates in web and Apple clients ([#290](https://github.com/cedricziel/folio/issues/290)) ([04db943](https://github.com/cedricziel/folio/commit/04db943cac8acec817419461681f8a471b161e91))
* **documents:** filter documents by review state and document date ([9048dcf](https://github.com/cedricziel/folio/commit/9048dcf7524566d402097d4a82cad9a56fcaf9cd))
* **documents:** persist explicit review decisions ([9fceb69](https://github.com/cedricziel/folio/commit/9fceb69bb629192f3fc42e1b7e432d5af2fb99c1))
* **documents:** persist explicit review decisions ([#295](https://github.com/cedricziel/folio/issues/295)) ([af82358](https://github.com/cedricziel/folio/commit/af82358416f99c3e06d9661ffc07ae8c32734c44))
* **documents:** record document attribution history ([052b16d](https://github.com/cedricziel/folio/commit/052b16d7c17f713cf7983b3685224a96049601d9))
* **documents:** record document attribution history ([#313](https://github.com/cedricziel/folio/issues/313)) ([e5f69d4](https://github.com/cedricziel/folio/commit/e5f69d4dcd56b0412b626f6f3d56ac9a90f1e5c4))
* **documents:** review and date controls in API, web and Apple clients ([#314](https://github.com/cedricziel/folio/issues/314)) ([771326f](https://github.com/cedricziel/folio/commit/771326f1554f12160ba5f2d6ec6ea28efc0dca0e))
* **events:** bound complete metadata value snapshots ([4eca1dd](https://github.com/cedricziel/folio/commit/4eca1dd9cd57ab1e2146c0c2ed8cb1715cf670e2))
* **events:** bound complete metadata value snapshots ([#296](https://github.com/cedricziel/folio/issues/296)) ([d4cdcae](https://github.com/cedricziel/folio/commit/d4cdcaecc4d034df020af7fbc22308e020767b1a))
* **events:** build metadata envelopes with shared mutation lineage ([35a3ef2](https://github.com/cedricziel/folio/commit/35a3ef2d2fca0d028fb9049d4a5a497be5ad7e32))
* **events:** preserve purge request and completion lineage ([34ccb51](https://github.com/cedricziel/folio/commit/34ccb518098cba157da6ef17bf0d47d38b5705b7))
* **events:** preserve purge request and completion lineage ([#289](https://github.com/cedricziel/folio/issues/289)) ([d8df7e6](https://github.com/cedricziel/folio/commit/d8df7e6fe8174da87219dd61a28c7df992811ba0))
* **events:** publish precise document metadata transitions ([#271](https://github.com/cedricziel/folio/issues/271)) ([ce6ac01](https://github.com/cedricziel/folio/commit/ce6ac01c5edee02bceac3743259e6c7ec2837f4a))
* **events:** publish vault-scoped document moves ([87dab4c](https://github.com/cedricziel/folio/commit/87dab4c8531e2648da1cbf6d36d4f342d5628fd9))
* **events:** publish vault-scoped document moves ([#284](https://github.com/cedricziel/folio/issues/284)) ([b04292f](https://github.com/cedricziel/folio/commit/b04292f723c1d82bfab4b22caac4192dade9f724))
* **infra:** persist saved views with transactional events ([df16679](https://github.com/cedricziel/folio/commit/df1667911ae2b0bcf0d718b4a5d6fd4a4e3cabef))
* **infra:** persist saved views with transactional events ([#275](https://github.com/cedricziel/folio/issues/275)) ([0ddd6a5](https://github.com/cedricziel/folio/commit/0ddd6a5c4e9ac49955e2815d36ecc4d429d48bc7))
* **infra:** persist stable metadata select options ([bf00087](https://github.com/cedricziel/folio/commit/bf00087372f47a75a8e112e21cf1de6c650f21c7))
* **infra:** persist stable metadata select options ([#294](https://github.com/cedricziel/folio/issues/294)) ([0038e97](https://github.com/cedricziel/folio/commit/0038e97d1713c8718167d0d97be09dca5d694c4d))
* **infra:** persist vault-scoped metadata fields ([43da880](https://github.com/cedricziel/folio/commit/43da8808abef0983ce10567f2f68d526e9ec011a))
* **infra:** persist vault-scoped metadata fields ([#285](https://github.com/cedricziel/folio/issues/285)) ([f48a638](https://github.com/cedricziel/folio/commit/f48a63820e05fde0ce247a513900591d10a76e19))
* **infra:** publish document metadata deltas transactionally ([a722013](https://github.com/cedricziel/folio/commit/a722013a2d1fb86e9a935b26bb6d0ea9a8a6bb91))
* **infra:** store processing stage runs ([116eee7](https://github.com/cedricziel/folio/commit/116eee7c139d7373ae7b6a1b7e868f5a88b65d27))
* **ingest:** persist outcomes with transactional events ([a0f50b8](https://github.com/cedricziel/folio/commit/a0f50b877e25cd46af58d58a765fef23ba5915be))
* **ingest:** persist outcomes with transactional events ([#293](https://github.com/cedricziel/folio/issues/293)) ([ff7c20e](https://github.com/cedricziel/folio/commit/ff7c20e3e199172592d8fa9ddae211d7172097e1))
* **ingest:** produce item outcomes and show them per source ([#309](https://github.com/cedricziel/folio/issues/309)) ([5298d05](https://github.com/cedricziel/folio/commit/5298d053e5a886be07eff216b1e26d8991f070bd))
* **ingest:** publish item outcomes and source health from the pipeline ([435563c](https://github.com/cedricziel/folio/commit/435563c7d5dd76db13640f54bbcc8747d2ae416d))
* **processing:** fence OCR stage transitions and recovery ([#307](https://github.com/cedricziel/folio/issues/307)) ([a794228](https://github.com/cedricziel/folio/commit/a7942282dd80a8c507fe352aa6c4130fea6de8d5))
* **queue:** fence OCR stage transitions and failure recovery ([7c6ba1d](https://github.com/cedricziel/folio/commit/7c6ba1d02d5247f167235db125e6dc4162f78978))
* **sources:** retain managed sources with lifecycle events ([ca0a1e7](https://github.com/cedricziel/folio/commit/ca0a1e7c1b8e39b112c7505c5f7599d9df2d1483))
* **sources:** retain managed sources with lifecycle events ([#311](https://github.com/cedricziel/folio/issues/311)) ([ab1abf3](https://github.com/cedricziel/folio/commit/ab1abf3240fecba12d0cb4d5e0366b096c173b4a))
* **storage:** add document entry ownership and retry receipts ([86d4451](https://github.com/cedricziel/folio/commit/86d44514237e92f95ec9256ed12be6b26fe521cb))
* **storage:** add document entry ownership and retry receipts ([#273](https://github.com/cedricziel/folio/issues/273)) ([644d300](https://github.com/cedricziel/folio/commit/644d300ee5471287601fc3e7783962109e9644d1))
* **storage:** persist document entries with atomic event receipts ([8d95743](https://github.com/cedricziel/folio/commit/8d957438b84cdb1022ec2f738d12b0b989596812))
* **storage:** persist document entries with atomic event receipts ([#277](https://github.com/cedricziel/folio/issues/277)) ([8711642](https://github.com/cedricziel/folio/commit/8711642db409ca775c3b4a19fbc6237556132197))
* **taxonomy:** retain classifications with audited trash and restore ([3240af8](https://github.com/cedricziel/folio/commit/3240af8110ad74f3d74d8046422aa4b2aadfe2d0))
* **taxonomy:** retain classifications with audited trash and restore ([#279](https://github.com/cedricziel/folio/issues/279)) ([6e5532f](https://github.com/cedricziel/folio/commit/6e5532fbc373e287ec89211c3a66bf9cb5d57d9e))
* **ui:** show document processing stages ([0f8575b](https://github.com/cedricziel/folio/commit/0f8575b4c2a2fdbc8a6f1ac89850d98cfbc162e2))
* **ui:** show document processing stages ([#281](https://github.com/cedricziel/folio/issues/281)) ([a3c5e02](https://github.com/cedricziel/folio/commit/a3c5e0264c979dd4ae730d698d678c55d57b4076))
* **vaults:** manage vault lifecycle and ownership ([29753ef](https://github.com/cedricziel/folio/commit/29753ef8d1df56d8704cd0a657d528de3eb7cd5a))
* **vaults:** manage vault lifecycle and ownership ([#317](https://github.com/cedricziel/folio/issues/317)) ([5d8b39a](https://github.com/cedricziel/folio/commit/5d8b39af2c90b8359602136ee663dd7cfd588d8c))
* **web:** add document notes and comments ([51e2008](https://github.com/cedricziel/folio/commit/51e2008fb21d3dcb0bde3321e9d747c0406ba914))
* **web:** add document notes and comments ([#287](https://github.com/cedricziel/folio/issues/287)) ([f9404ff](https://github.com/cedricziel/folio/commit/f9404ff664458b6249c991e14bd6077e3e85067a))
* **web:** add review controls and review and date filters ([b7d106c](https://github.com/cedricziel/folio/commit/b7d106c43bae68bdc4c5b6a8fdbafc3389fddfae))
* **web:** save and restore document views ([f78a0e5](https://github.com/cedricziel/folio/commit/f78a0e5e70acea02e23199eefe115bffce0ae729))
* **web:** save and restore document views ([#282](https://github.com/cedricziel/folio/issues/282)) ([3ff723f](https://github.com/cedricziel/folio/commit/3ff723f93e59cf5c4e6f83a54b010fbedad0185d))
* **worker:** run claimed OCR through stage fences ([1b76b30](https://github.com/cedricziel/folio/commit/1b76b30ed8a551cca8e2432ab9132a74c4d8a9f5))
* **workflows:** add bounded condition groups ([f84f47f](https://github.com/cedricziel/folio/commit/f84f47f2d8f44af77e8bc6ad8da08e814988bf72))
* **workflows:** add bounded condition groups ([#272](https://github.com/cedricziel/folio/issues/272)) ([a1b6bd5](https://github.com/cedricziel/folio/commit/a1b6bd54882bd05c71eab25468292ea0754bea9c))
* **workflows:** add typed trigger registration ([dfff5bb](https://github.com/cedricziel/folio/commit/dfff5bb63d231f2459d9267b6ed304264eacc14d))
* **workflows:** add typed trigger registration ([#280](https://github.com/cedricziel/folio/issues/280)) ([cc9ebff](https://github.com/cedricziel/folio/commit/cc9ebff0d9246bc5f93d857b68cb009f4738c164))
* **workflows:** capture event conditions and deduplicate mutations ([#299](https://github.com/cedricziel/folio/issues/299)) ([0bb21e9](https://github.com/cedricziel/folio/commit/0bb21e9425c371d4d88f1ed6bcb6682c51be89c2))
* **workflows:** deduplicate queued runs by captured mutation ([32cb0f7](https://github.com/cedricziel/folio/commit/32cb0f7820fc9f4fd6eecc07db720efdd21e54a6))
* **workflows:** dispatch precise metadata events ([10d8469](https://github.com/cedricziel/folio/commit/10d8469f3c731eb2edbfd231c9714dfcd63671e8))
* **workflows:** dispatch precise metadata events ([#291](https://github.com/cedricziel/folio/issues/291)) ([b00ab13](https://github.com/cedricziel/folio/commit/b00ab136be193b9fa0e2bed0eb096fda649d397d))
* **workflows:** match captured event subject and actor conditions ([152baa1](https://github.com/cedricziel/folio/commit/152baa125ea4769fb5dec640e235e40a1f5cdb57))
* **workflows:** retain workflow configurations with audited history ([39ba831](https://github.com/cedricziel/folio/commit/39ba831dfab1efff138b3e81a8eee298de2b690e))
* **workflows:** retain workflow configurations with audited history ([#312](https://github.com/cedricziel/folio/issues/312)) ([44a9d5a](https://github.com/cedricziel/folio/commit/44a9d5add94a739719986f82a15c40f0dbc68275))


### Bug Fixes

* **api:** describe entry success responses in OpenAPI ([e59137b](https://github.com/cedricziel/folio/commit/e59137b46fd1ae5ab32fdfacb280f53da19e8fe4))
* **api:** keep page size on outcome links and log load failures ([89a0906](https://github.com/cedricziel/folio/commit/89a09065d171dd873bf384fa366e537688bf4cf1))
* **api:** log outcome load errors with Debug and declare page bounds ([ae7929f](https://github.com/cedricziel/folio/commit/ae7929fe5889eaeba1a6b38994595474a8d325ec))
* **apple:** assign the source activity error on the view model ([0efe253](https://github.com/cedricziel/folio/commit/0efe25385f2434f6af2caefc7f3a25e0673f2271))
* **apple:** build classification management in the UI test fixtures ([5a4dcf1](https://github.com/cedricziel/folio/commit/5a4dcf129317b96f10ede21ebbc1204f384c81f5))
* **apple:** drop refresh results that a sign-out or newer sign-in superseded ([f894a84](https://github.com/cedricziel/folio/commit/f894a841360703f78cd49354dbe6d0f5f02be908))
* **apple:** drop refresh results that a sign-out or newer sign-in superseded ([#302](https://github.com/cedricziel/folio/issues/302)) ([3b07f42](https://github.com/cedricziel/folio/commit/3b07f424781fdad6010dbd29daf06e98544ebaea))
* **apple:** drop the duplicate fixture scope and clarify sheet checks ([b884c62](https://github.com/cedricziel/folio/commit/b884c62e408650647d07c9a829d9dac7d76e244d))
* **apple:** keep a saved document date when a slower refresh returns ([07c5a9e](https://github.com/cedricziel/folio/commit/07c5a9e3ef84ec3a9526d89731abb5690e6320e9))
* **apple:** keep trashed classification names and refresh them after edits ([35ab205](https://github.com/cedricziel/folio/commit/35ab205e1d182398a4423a62436fd1745197f5a3))
* **apple:** recover discussion drafts from rejected and stale writes ([65d53ac](https://github.com/cedricziel/folio/commit/65d53ac52ea7021fcf5b0d9fcf346c8106c639dc))
* **apple:** report unreadable classification responses through the shared call path ([f6d100f](https://github.com/cedricziel/folio/commit/f6d100f7f13c6811550b4f3e3370a9c66e1e3082))
* **apple:** trim classification names and label additions per entry ([5dde2d2](https://github.com/cedricziel/folio/commit/5dde2d223077754c5cfa545139cc38d6a1a2d22d))
* **auth:** require a vault management scope for bearer vault changes ([caec7e9](https://github.com/cedricziel/folio/commit/caec7e9bcb3846d1286c7c5e9ca905887171bf80))
* **auth:** revoke pending authorization codes when disconnecting a client ([685a930](https://github.com/cedricziel/folio/commit/685a930915d7bd139d400e5449549769f6491942)), closes [#163](https://github.com/cedricziel/folio/issues/163)
* **auth:** revoke pending authorization codes when disconnecting a client ([#303](https://github.com/cedricziel/folio/issues/303)) ([a3187ed](https://github.com/cedricziel/folio/commit/a3187edecd8b455adb5691848c10f70aa12c4b15)), closes [#163](https://github.com/cedricziel/folio/issues/163)
* **core:** bound URLs by their normalized length ([905330d](https://github.com/cedricziel/folio/commit/905330d222fa80e3996d6dda7f5995932bafb758))
* **core:** require an explicit value in metadata changes ([4a8c3a2](https://github.com/cedricziel/folio/commit/4a8c3a2154f7b1c2abc3a6f1485e019deb85ebe2))
* **db:** surface stale and deadlocked metadata writes as conflicts ([be0cc1e](https://github.com/cedricziel/folio/commit/be0cc1e2feb829691148a3cc4b827db880d8d5c2))
* **documents:** attribute actorless unreviews to the system ([a77f25e](https://github.com/cedricziel/folio/commit/a77f25e90e79ba76db559dc9631d932a5a3e71d6))
* **documents:** attribute review decisions to the reviewer ([3b443cd](https://github.com/cedricziel/folio/commit/3b443cd0d78b23eb53b0da3d11d64b5ce35d61b0))
* **documents:** keep attribution on status-only changes ([a5ace71](https://github.com/cedricziel/folio/commit/a5ace716beb4d2b6c63e60d7f68cb995aad8edf7))
* **documents:** refuse moves into deleted vaults ([0b2761d](https://github.com/cedricziel/folio/commit/0b2761d9fd351004520058549d1a59a8cc21c3ca))
* **ingest:** keep sources running when a vault lookup fails ([3a69d5c](https://github.com/cedricziel/folio/commit/3a69d5c67bfdbb754ce00ddc66c3654453ed359f))
* **ingest:** let active sources renew claims left by deleted sources ([431819c](https://github.com/cedricziel/folio/commit/431819c7798b11351beb54dbc0ca86eb27910711))
* **ingest:** reject observed items for a vault deleted mid-poll ([0c7854d](https://github.com/cedricziel/folio/commit/0c7854db14e66892445ea6cde55b0b9e5196e88e))
* **ingest:** restart sources whose poll task stopped ([85ae648](https://github.com/cedricziel/folio/commit/85ae6481cd1d66e4cd858584d1c2f1e7e234f511))
* **ingest:** store a fixed reason instead of raw source errors ([a07c68f](https://github.com/cedricziel/folio/commit/a07c68fc093aa3209a5d7710a219f5c0f8b511c9))
* **mcp:** detach retained deleted tags by name ([6655abd](https://github.com/cedricziel/folio/commit/6655abde883b1d7f01f72898261de0f7e59f6906))
* **processing:** commit OCR completion with its event ([df2c014](https://github.com/cedricziel/folio/commit/df2c014b25dffc610b85ff25e8ef788685d6b2ad))
* **processing:** commit OCR completion with its event ([#274](https://github.com/cedricziel/folio/issues/274)) ([3aa4d1f](https://github.com/cedricziel/folio/commit/3aa4d1f3be37c7fd49df0524ab152534f0aef9d6))
* **processing:** hold the vault while completing OCR and skip deleted vaults in repair ([b30a56d](https://github.com/cedricziel/folio/commit/b30a56dca7df84c5bd1ea878901d9e957f3b6328))
* **processing:** keep OCR results when a newer group owns embedding ([5fd18f0](https://github.com/cedricziel/folio/commit/5fd18f0c693b6b0a1215dd91bff630613be96965))
* **processing:** reject invalid embedding batches ([#268](https://github.com/cedricziel/folio/issues/268)) ([b40a353](https://github.com/cedricziel/folio/commit/b40a353b59ab5641d1757aa3777618951801fb12))
* **processing:** respect disabled embedding on OCR completion ([4f9eecb](https://github.com/cedricziel/folio/commit/4f9eecbb5146ea07f75e5d03f9d0e5c98bb1b956))
* **processing:** retain queued scope and event lineage ([30fbe4e](https://github.com/cedricziel/folio/commit/30fbe4e06d1f4f9b2cdf127943881004c6c13051))
* **processing:** retain queued scope and event lineage ([#288](https://github.com/cedricziel/folio/issues/288)) ([d56afe4](https://github.com/cedricziel/folio/commit/d56afe42527bd5c19fdda48665e76b24daf92265))
* **processing:** skip document work in deleted vaults ([a3fc41f](https://github.com/cedricziel/folio/commit/a3fc41ff2dbc2e8e492785e3186324824b83d0cf))
* **queue:** keep recovering expired jobs after one fails ([83b0729](https://github.com/cedricziel/folio/commit/83b0729f329ad043b1a6f4c994ffd5ddf5db7552))
* **sqlite:** acquire write lock before trash snapshot ([7a6520c](https://github.com/cedricziel/folio/commit/7a6520c95479bf10a99c9bac2d59ff5e6f6b0c05))
* **sqlite:** acquire write lock before trash snapshot ([#298](https://github.com/cedricziel/folio/issues/298)) ([68c9723](https://github.com/cedricziel/folio/commit/68c9723fdd3d584a6ac25c35f87bc8399ce16338))
* **taxonomy:** keep retained tags when replacing a document's tags ([5772daa](https://github.com/cedricziel/folio/commit/5772daaaad2cbc5530d460ab799c5e36c3a98795))
* **ui:** keep extraction recovery guidance in one place ([949995e](https://github.com/cedricziel/folio/commit/949995e4037eb6556a1f372f1850bdd284115c5e))
* **vaults:** attribute provisioned vaults and ignore other members' defaults ([9e3536f](https://github.com/cedricziel/folio/commit/9e3536fb10985e7a63b4e9fb9af84e4758fb564b))
* **vaults:** bootstrap the instance default without a lifecycle event ([b6b084e](https://github.com/cedricziel/folio/commit/b6b084e90072520379b2d0e1db20b1ce530ca106))
* **vaults:** limit member lists to owners ([657f430](https://github.com/cedricziel/folio/commit/657f430a20daf4a2de26b9b4a01a1646894a96c6))
* **vaults:** satisfy lint and the hidden sidebar counts after merging main ([005619d](https://github.com/cedricziel/folio/commit/005619dc1e661e3ff669fb5d913cd0b47f9a654e))
* **web:** keep review filters in saved views and reviewed inbox results ([72d23a6](https://github.com/cedricziel/folio/commit/72d23a6efd3d4b3fa7b4be90d945dbd881c1b29b))
* **web:** keep saved entries visible across pagination ([cdf9c57](https://github.com/cedricziel/folio/commit/cdf9c570514d26c4194bf5adac97bdb61eda0c8c))
* **web:** stack source rows on narrow screens ([7f5fef2](https://github.com/cedricziel/folio/commit/7f5fef2158000cd2dedda15ceefea6d1f5239efa))
* **workflows:** drop the stray run status binding ([2e82aff](https://github.com/cedricziel/folio/commit/2e82affaa0e8d11fc0a55473b0935fe3e924e366))
* **workflows:** stop in-flight runs once their workflow is deleted ([bc021e9](https://github.com/cedricziel/folio/commit/bc021e9ddaeecae2e30bdb2a86109d51cc4159e9))


### Performance Improvements

* **events:** share live events across SSE subscribers ([156d786](https://github.com/cedricziel/folio/commit/156d786e30abdfc94a1e376d10be4d9a7900c2c6))

## [0.8.0](https://github.com/cedricziel/folio/compare/v0.7.0...v0.8.0) (2026-11-02)


### Features

* **api:** manage and enforce personal token permissions ([bc487d1](https://github.com/cedricziel/folio/commit/bc487d135f3d436b818729dde01f88362328fd69))
* **api:** serve authenticated document thumbnails ([e7702c2](https://github.com/cedricziel/folio/commit/e7702c253818724b16d6d3b0c15ccea4e462b22f))
* **api:** serve authenticated document thumbnails ([#247](https://github.com/cedricziel/folio/issues/247)) ([56bdb1b](https://github.com/cedricziel/folio/commit/56bdb1b1e99e9a821222b139e9bc69a4cc468f5f))
* **apple:** cache and display document row thumbnails ([44d407d](https://github.com/cedricziel/folio/commit/44d407d2c91135c3128d6f17ffa703f61a2372c0))
* **apple:** cache and display document row thumbnails ([#255](https://github.com/cedricziel/folio/issues/255)) ([bf05971](https://github.com/cedricziel/folio/commit/bf059719ea8184418296e8d6459de178330468ab))
* **apple:** provide authenticated File Provider thumbnails ([b40d510](https://github.com/cedricziel/folio/commit/b40d510def63ea4cbc6887e02b48deec3caf79de))
* **apple:** provide authenticated File Provider thumbnails ([#253](https://github.com/cedricziel/folio/issues/253)) ([2da7bff](https://github.com/cedricziel/folio/commit/2da7bff5829fdcff2466924298705094e37edf03))
* **apple:** retain document lists beside wide previews ([895a693](https://github.com/cedricziel/folio/commit/895a69306099a05f8e73f0f719bfd036a2fbf9b8))
* **apple:** retain document lists beside wide previews ([#228](https://github.com/cedricziel/folio/issues/228)) ([08e4af9](https://github.com/cedricziel/folio/commit/08e4af92fd45978c947eae4236d2dd3b78d892e6))
* **auth:** add scoped personal access tokens ([072251f](https://github.com/cedricziel/folio/commit/072251f22802d82aa3ad1e90c93bc07f2819ef85))
* **core:** define file identity and revision storage keys ([3a6a96d](https://github.com/cedricziel/folio/commit/3a6a96dbfa8e797f3e4e0405499f3f7393b4bed1))
* **core:** define file identity and revision storage keys ([#263](https://github.com/cedricziel/folio/issues/263)) ([e8958ea](https://github.com/cedricziel/folio/commit/e8958ea78c25caf59b6a3a0a968bf00be9bbae57))
* **core:** define versioned saved-view contracts ([1bd34fa](https://github.com/cedricziel/folio/commit/1bd34fa65647f04cd3041e50aacf76467a16d501))
* **core:** define versioned saved-view contracts ([#264](https://github.com/cedricziel/folio/issues/264)) ([48a49a6](https://github.com/cedricziel/folio/commit/48a49a628551ee8e41a0a3d04520abfc72de7fec))
* **core:** derive precise document metadata transitions ([17e025a](https://github.com/cedricziel/folio/commit/17e025a5246342ad13b72a5e8076974a12fcd272))
* **core:** derive precise document metadata transitions ([#261](https://github.com/cedricziel/folio/issues/261)) ([3355716](https://github.com/cedricziel/folio/commit/3355716bf4a5fb7f3e676e73785994b8c0aaf7d2))
* **core:** model document dates and explicit review decisions ([777b949](https://github.com/cedricziel/folio/commit/777b9492ae4626be426088cc591e1990536f1ff5))
* **core:** model document dates and explicit review decisions ([#262](https://github.com/cedricziel/folio/issues/262)) ([b650ecc](https://github.com/cedricziel/folio/commit/b650ecc51db5cf2b62dd321a528da3d0c4378b3e))
* **core:** require explicit vault ownership on domain models ([7e7e010](https://github.com/cedricziel/folio/commit/7e7e0107e4d83ac9d3653598968a2b4bf62781a8))
* **core:** require explicit vault ownership on domain models ([#260](https://github.com/cedricziel/folio/issues/260)) ([e635500](https://github.com/cedricziel/folio/commit/e635500b50001842992f2259d1d5f702fc257d04))
* **core:** scope mutation actors to authenticated requests ([c801b46](https://github.com/cedricziel/folio/commit/c801b46b2a7426562d40bdc2be895e9ac00bc9b7))
* **core:** scope mutation actors to authenticated requests ([#258](https://github.com/cedricziel/folio/issues/258)) ([3358bb3](https://github.com/cedricziel/folio/commit/3358bb39208e4970eb78eaa71fa457cdd486391a))
* **core:** validate typed metadata values exactly ([8f5cd47](https://github.com/cedricziel/folio/commit/8f5cd47e9275bdea673802db60147fbfe6a686e8))
* **core:** validate typed metadata values exactly ([#259](https://github.com/cedricziel/folio/issues/259)) ([d356183](https://github.com/cedricziel/folio/commit/d35618363509284c41bed910aca362d07c7f158b))
* **events:** add versioned transactional domain envelopes ([f2072e0](https://github.com/cedricziel/folio/commit/f2072e0ca23c5b98596885e98627e60e70b4bb82))
* **events:** add versioned transactional domain envelopes ([#265](https://github.com/cedricziel/folio/issues/265)) ([154c484](https://github.com/cedricziel/folio/commit/154c484d943dd50cd9d6086f84122678646435e8))
* **integrations:** add shortcuts and scoped personal tokens ([#251](https://github.com/cedricziel/folio/issues/251)) ([2ff04be](https://github.com/cedricziel/folio/commit/2ff04be693d61cabf40d95c56c210812ddc6119c))
* **web:** add integration shortcuts and token controls ([f7d6dc4](https://github.com/cedricziel/folio/commit/f7d6dc43e88a77660dd41d8f2eab83d2f3baf97a))


### Bug Fixes

* **apple:** expose Mac settings and account commands ([#227](https://github.com/cedricziel/folio/issues/227)) ([080e948](https://github.com/cedricziel/folio/commit/080e94878a1f968df67def9e8a77befd505a74c3))
* **apple:** preserve the filename in shared document titles ([de6a73e](https://github.com/cedricziel/folio/commit/de6a73ecc8761a1809271b17474e3549e2abbbd0))
* **apple:** preserve the filename in shared document titles ([#248](https://github.com/cedricziel/folio/issues/248)) ([29ca4f6](https://github.com/cedricziel/folio/commit/29ca4f6672adecd4e72652f43882ee2c97c97b76))
* **auth:** preserve browser session authentication ([816482a](https://github.com/cedricziel/folio/commit/816482a5bc4f3344154b2e243caed982598192fa))
* **events:** preserve origin and metadata through delayed delivery ([d1703f4](https://github.com/cedricziel/folio/commit/d1703f48472c52c6a6212dceff44e7dc79e63572))
* **events:** preserve origin and metadata through delayed delivery ([#252](https://github.com/cedricziel/folio/issues/252)) ([a0671ba](https://github.com/cedricziel/folio/commit/a0671ba0429f9f027013cb79b8cb6355ac779d53))
* **ingest:** bound IMAP operations and discard timed-out sessions ([110fb6d](https://github.com/cedricziel/folio/commit/110fb6d470f7e4b29610defe0fca3dc8385ab0b6))
* **ingest:** bound IMAP operations and discard timed-out sessions ([#243](https://github.com/cedricziel/folio/issues/243)) ([ef53979](https://github.com/cedricziel/folio/commit/ef539790daf75128b8e73b41b5d3b0cd7583fabf))
* **ingest:** claim watched files before reading and cleanup ([f149e3c](https://github.com/cedricziel/folio/commit/f149e3c3291f52e1ddea669fd1cde4269393b044))
* **ingest:** claim watched files before reading and cleanup ([#240](https://github.com/cedricziel/folio/issues/240)) ([6739c4b](https://github.com/cedricziel/folio/commit/6739c4b5c8e8da6c5ff0e108abe7c07fbcdb5f1c)), closes [#145](https://github.com/cedricziel/folio/issues/145)
* **ingest:** preserve inputs while matching imports are pending ([#231](https://github.com/cedricziel/folio/issues/231)) ([67652ef](https://github.com/cedricziel/folio/commit/67652ef977bc054853d536f5cd5b6f46f12e5a8a)), closes [#147](https://github.com/cedricziel/folio/issues/147)
* **ingest:** publish creation only after original finalization ([f6d6ea5](https://github.com/cedricziel/folio/commit/f6d6ea53fae5f7c3d771d36290b026087d2f76ae))
* **ingest:** publish creation only after original finalization ([#244](https://github.com/cedricziel/folio/issues/244)) ([8fc9626](https://github.com/cedricziel/folio/commit/8fc9626ed4131eddfe51269454a1809598e42677))
* **ingest:** recover interrupted imports with fenced document identity ([58edf93](https://github.com/cedricziel/folio/commit/58edf93d12dcaf9beddc0b451b627ee9d11a1cf7))
* **ingest:** recover interrupted imports with fenced document identity ([#242](https://github.com/cedricziel/folio/issues/242)) ([c52756f](https://github.com/cedricziel/folio/commit/c52756f2c2de25e1969969eb6eb87176aa1fa31f))
* **ingest:** scope fallback mail identity to mailbox validity ([#234](https://github.com/cedricziel/folio/issues/234)) ([941ea7f](https://github.com/cedricziel/folio/commit/941ea7fa686872217938522d7ec30e4a6431bc54)), closes [#146](https://github.com/cedricziel/folio/issues/146)
* **integrations:** align tokens with current API and migrations ([cb8f925](https://github.com/cedricziel/folio/commit/cb8f92514fe697f818937177e10ff2f979032252))
* **mcp:** authorize vocabulary creation at the write boundary ([96f69c0](https://github.com/cedricziel/folio/commit/96f69c061300168b9a2dc5158cd02138daaa99fb))
* **mcp:** enforce vault permissions and session ownership ([a2970c9](https://github.com/cedricziel/folio/commit/a2970c9ddc4c6cee167b704753fd6eed52ec0653))
* **outbox:** persist immutable document context with each event ([e237564](https://github.com/cedricziel/folio/commit/e2375649d5082cca0aa6eeef01ba903308a7ff0f))
* **outbox:** persist immutable document context with each event ([#246](https://github.com/cedricziel/folio/issues/246)) ([00ec34d](https://github.com/cedricziel/folio/commit/00ec34d04c874bf025219834ee81b85aa6fde817))
* **queue:** protect active jobs with renewable leases ([#237](https://github.com/cedricziel/folio/issues/237)) ([c6a49af](https://github.com/cedricziel/folio/commit/c6a49aff511849303ab33e817ee523e9ae4e1e9a)), closes [#138](https://github.com/cedricziel/folio/issues/138)
* **ui:** complete scoped interface transitions ([#223](https://github.com/cedricziel/folio/issues/223)) ([d066e20](https://github.com/cedricziel/folio/commit/d066e201a1fba57bfb2d4aaedafa8017bb43fdb3))
* **ui:** identify upload retry buttons by filename ([8acfb22](https://github.com/cedricziel/folio/commit/8acfb22c4257879066ffa3eee488b2979b6abd02))
* **ui:** separate upload dismissal from cancellation ([#241](https://github.com/cedricziel/folio/issues/241)) ([43c6484](https://github.com/cedricziel/folio/commit/43c6484c8470166818e4d1b7b918e25198da82b6))
* **web:** unify input styling across forms ([dea2e08](https://github.com/cedricziel/folio/commit/dea2e080d973cd54f40d6f928711144c25500559))
* **web:** unify input styling across forms ([#249](https://github.com/cedricziel/folio/issues/249)) ([cc1ef63](https://github.com/cedricziel/folio/commit/cc1ef63731ce5d55f5e40ed502748baab3eec766))

## [0.7.0](https://github.com/cedricziel/folio/compare/v0.6.0...v0.7.0) (2026-11-02)


### ⚠ BREAKING CHANGES

* **apple:** require vault-aware servers and identifiers ([#220](https://github.com/cedricziel/folio/issues/220))
* **apple:** require vault-aware servers and identifiers
* remove runtime backward-compatibility paths ([#206](https://github.com/cedricziel/folio/issues/206))
* **apple:** remove installation identity migration
* **auth:** require resource-specific MCP metadata discovery
* **vaults:** remove automatic archive claims
* **storage:** remove support for previous object layouts
* **config:** remove obsolete provider and environment aliases

### Features

* **api:** combine literal search with paged document filters ([ac54777](https://github.com/cedricziel/folio/commit/ac547777ebf530827050c56b9c199a15334e5e37))
* **api:** combine literal search with paged document filters ([#208](https://github.com/cedricziel/folio/issues/208)) ([fa3792e](https://github.com/cedricziel/folio/commit/fa3792e4a0b86c642b33d41602ccd30f25e5d58e))
* **api:** expose authorized paged document browsing ([3c7000a](https://github.com/cedricziel/folio/commit/3c7000a7ce7d78d802d281ec14d4770db1d4674b))
* **api:** expose authorized paged document browsing ([#205](https://github.com/cedricziel/folio/issues/205)) ([32e9f6c](https://github.com/cedricziel/folio/commit/32e9f6c3f34612f20418ceed618d0ce3ee20ee22))
* **apple:** add document drag-and-drop imports ([f07f6e9](https://github.com/cedricziel/folio/commit/f07f6e91ea45285dcf6e4edce6ec619d0fd81ec7))
* **apple:** add document drag-and-drop imports ([#219](https://github.com/cedricziel/folio/issues/219)) ([b079f30](https://github.com/cedricziel/folio/commit/b079f30e91eb447f8365c4fa0531ebd6d19cacfa))
* **apple:** add optional biometric app locking ([4c741c4](https://github.com/cedricziel/folio/commit/4c741c41d21b197db471d246e732f54a72ff8ba2))
* **apple:** add optional biometric app locking ([#216](https://github.com/cedricziel/folio/issues/216)) ([5639e38](https://github.com/cedricziel/folio/commit/5639e38dece475612863c4b7f76e2e4fb874cd8b))
* **apple:** browse documents with bounded pages ([53c35d8](https://github.com/cedricziel/folio/commit/53c35d8765a3ecd6e8946a6835608aab021471d8))
* **apple:** browse documents with bounded pages ([#218](https://github.com/cedricziel/folio/issues/218)) ([ed98bf7](https://github.com/cedricziel/folio/commit/ed98bf78cfa13c49a02b04beadaa1d298269276a))
* **apple:** restore trashed documents with native undo ([5889989](https://github.com/cedricziel/folio/commit/5889989eafbf204d680e219bce9c8199a0ef3b8b))
* **apple:** restore trashed documents with native undo ([#226](https://github.com/cedricziel/folio/issues/226)) ([a306c0b](https://github.com/cedricziel/folio/commit/a306c0b28beb6e0fda63121865228e8b96647174))
* **apple:** scope vault browsing and file provider folders ([db5eb16](https://github.com/cedricziel/folio/commit/db5eb16eebe115f0f00ce27262205e2e828c5d6a))
* **documents:** make deletion recoverable with Trash ([#217](https://github.com/cedricziel/folio/issues/217)) ([08a7fb0](https://github.com/cedricziel/folio/commit/08a7fb0c40ec266f4c4133ea844bc8890179b0ab))
* **documents:** move deleted documents to recoverable trash ([2d032d5](https://github.com/cedricziel/folio/commit/2d032d5ab6fbbd6b61eb4d520141993a4c3507f5))
* **vaults:** add library selection across web and Apple ([#210](https://github.com/cedricziel/folio/issues/210)) ([376887a](https://github.com/cedricziel/folio/commit/376887aeba7280a6d3db14ae2cdb39b47438edb4))
* **vaults:** add scoped library selection to the web app ([aa0e40c](https://github.com/cedricziel/folio/commit/aa0e40c6503ce5a2f72fe10a0e16b9ad57c4e6d9))


### Bug Fixes

* **api:** bound literal search term count ([0abc45d](https://github.com/cedricziel/folio/commit/0abc45d8b92eba2f85cb6006c1135d7a1c1b487a))
* **apple:** protect scans and resume partial uploads ([3651a2c](https://github.com/cedricziel/folio/commit/3651a2cfb9559c9007099655787f510e782b5d39))
* **apple:** recover sessions without losing the selected server ([ce4514a](https://github.com/cedricziel/folio/commit/ce4514aebc3e7a021f894c136cee0aab6b82731d))
* **apple:** recover sessions without losing the selected server ([#221](https://github.com/cedricziel/folio/issues/221)) ([01288b4](https://github.com/cedricziel/folio/commit/01288b442598adcda75a3809e6a9a4980a54809a))
* **apple:** retain attachment failures and original filenames ([3361ff0](https://github.com/cedricziel/folio/commit/3361ff082a4040654b33ecb51ec61d074fba9bb7))
* **apple:** retain attachment failures and original filenames ([#230](https://github.com/cedricziel/folio/issues/230)) ([f56ba08](https://github.com/cedricziel/folio/commit/f56ba08065d4ebbc0e449ea34ecd6824e315ac56)), closes [#167](https://github.com/cedricziel/folio/issues/167) [#168](https://github.com/cedricziel/folio/issues/168)
* **apple:** route inbox taps through the document path ([4289e70](https://github.com/cedricziel/folio/commit/4289e7051b2b72e019b9a0431c17c15e4c95e9ba))
* **apple:** route inbox taps through the document path ([#212](https://github.com/cedricziel/folio/issues/212)) ([523bd05](https://github.com/cedricziel/folio/commit/523bd05ecd8e2e0329572cc839d1c4ed39296e4c))
* **apple:** support increased contrast in custom colors ([d74b0ce](https://github.com/cedricziel/folio/commit/d74b0ce3c9fe9932dce6585c49e3f1c178c97962))
* **auth:** reject mismatched forwarded sessions ([88e9dc6](https://github.com/cedricziel/folio/commit/88e9dc6166a127e958ec3f2c3a861385ee55cbbe))
* **auth:** reject mismatched forwarded sessions ([#232](https://github.com/cedricziel/folio/issues/232)) ([707b29a](https://github.com/cedricziel/folio/commit/707b29a24ec650f06141fcc7e71764e395743779))
* **auth:** replace deprecated XNonce conversions ([8ebabb2](https://github.com/cedricziel/folio/commit/8ebabb2e2e7c1560ac0c6fa2ab167a694ce474bb))
* **ci:** allow cold simulator startup in navigation tests ([a8f6f02](https://github.com/cedricziel/folio/commit/a8f6f0282c1aeb6243a14640e9c4b39472621719))
* **config:** retain global AI provider configuration ([f47c34b](https://github.com/cedricziel/folio/commit/f47c34b841537e640b528015c45b646cc67d4999))
* **ingest:** restrict IMAP cleanup to imported messages ([1051f37](https://github.com/cedricziel/folio/commit/1051f37ce03defeb4583ecffe489032cda914eef))
* **ingest:** restrict IMAP cleanup to imported messages ([#211](https://github.com/cedricziel/folio/issues/211)) ([ea134e2](https://github.com/cedricziel/folio/commit/ea134e26a3c1e407c0becd3e60a408b30a5d2b6e))
* **mcp:** bound original file reads before buffering ([3e79634](https://github.com/cedricziel/folio/commit/3e796347c9fe91d984132a6396a9cd4b3b66dbc4))
* **mcp:** bound original file reads before buffering ([#233](https://github.com/cedricziel/folio/issues/233)) ([6214f7e](https://github.com/cedricziel/folio/commit/6214f7ea2ff2e9d59622a183c344eb0d061cd9cc)), closes [#139](https://github.com/cedricziel/folio/issues/139)
* **settings:** restrict global controls to administrators ([ceae988](https://github.com/cedricziel/folio/commit/ceae988a43d9021b24f6b0bb746e7d38e04a961a))
* **settings:** restrict global controls to administrators ([#235](https://github.com/cedricziel/folio/issues/235)) ([52bdc10](https://github.com/cedricziel/folio/commit/52bdc101b7505f1e7d331f8f8044d8f2efb8444b))
* **ui:** ignore stale command palette results ([9488758](https://github.com/cedricziel/folio/commit/94887588d03405bffbcbde2d4434c09196edf4fa))
* **ui:** ignore stale command palette results ([#239](https://github.com/cedricziel/folio/issues/239)) ([ff479ec](https://github.com/cedricziel/folio/commit/ff479ecf4edb5aa3ed3f2e29eee7716db1b12e99))
* **ui:** render document tags with DOM APIs ([585fa79](https://github.com/cedricziel/folio/commit/585fa790553234c9a04407f11c753a7876a7d23d))
* **ui:** render document tags with DOM APIs ([#236](https://github.com/cedricziel/folio/issues/236)) ([fdb5152](https://github.com/cedricziel/folio/commit/fdb5152da50307422082fc7ea258a80a1df44fce))


### Code Refactoring

* **apple:** remove installation identity migration ([15e8f26](https://github.com/cedricziel/folio/commit/15e8f26db6c12d1e905de2c9a3a221b0fbeb2f0f))
* **apple:** require vault-aware servers and identifiers ([eb8fd0e](https://github.com/cedricziel/folio/commit/eb8fd0e89b870149121371d4253f17a40c839486))
* **apple:** require vault-aware servers and identifiers ([#220](https://github.com/cedricziel/folio/issues/220)) ([ac17501](https://github.com/cedricziel/folio/commit/ac17501f4d5691c015c9ef174717ea90ebb65457))
* **auth:** require resource-specific MCP metadata discovery ([bf76628](https://github.com/cedricziel/folio/commit/bf76628dc2d3804843904179f3c9161583065b4e))
* **config:** remove obsolete provider and environment aliases ([f3e17c4](https://github.com/cedricziel/folio/commit/f3e17c46d1c20ac4dd306bdb774b949b5f46f50b))
* remove runtime backward-compatibility paths ([#206](https://github.com/cedricziel/folio/issues/206)) ([73f274e](https://github.com/cedricziel/folio/commit/73f274e89e30fa7f5e81bfdf81ed4d44249921cd))
* **storage:** remove support for previous object layouts ([035bfaa](https://github.com/cedricziel/folio/commit/035bfaa58ec1aefaf710141aca7cab641f98f0df))
* **vaults:** remove automatic archive claims ([c28f269](https://github.com/cedricziel/folio/commit/c28f26979855aa6b3c90386d1a72d379c5048cb8))

## [0.6.0](https://github.com/cedricziel/folio/compare/v0.5.0...v0.6.0) (2026-11-02)


### Features

* **apple:** add library workspace navigation ([2b12762](https://github.com/cedricziel/folio/commit/2b12762505281d5b0c9f4ca516a1ec724bbbd0b0))
* **apple:** add library workspace navigation ([#195](https://github.com/cedricziel/folio/issues/195)) ([cfdd0be](https://github.com/cedricziel/folio/commit/cfdd0bea147e688d0994312f278988a938985151))
* **infra:** add bounded document browse queries ([55cb15f](https://github.com/cedricziel/folio/commit/55cb15feef720abe5e19129c326f5e89b3f7434b))
* **infra:** add bounded document browse queries ([#193](https://github.com/cedricziel/folio/issues/193)) ([642aa16](https://github.com/cedricziel/folio/commit/642aa166f246536d2da0d4a104bf2562d0c4f86d))


### Bug Fixes

* **apple:** keep previews alive across navigation cancellation ([ff5b496](https://github.com/cedricziel/folio/commit/ff5b4969c48d6fbd85609af1a1def6829c5b226e))
* **apple:** keep previews alive across navigation cancellation ([#196](https://github.com/cedricziel/folio/issues/196)) ([9303974](https://github.com/cedricziel/folio/commit/9303974044da0bcd2145eec8903747e3119956a7))


### Performance Improvements

* **api:** batch document tag enrichment ([2126fc8](https://github.com/cedricziel/folio/commit/2126fc8d8b315eb088a5171a8abeb1b8b2ca1c0b))
* **api:** batch document tag enrichment ([#204](https://github.com/cedricziel/folio/issues/204)) ([64297c1](https://github.com/cedricziel/folio/commit/64297c13e206f9304c7ab8bf71e79af35490f389))

## [0.5.0](https://github.com/cedricziel/folio/releases/tag/v0.5.0) (2026-11-02)

This release expands local document processing and gives webhook actions distinct delivery identities.

### Before you upgrade

> [!WARNING]
> **Webhook delivery IDs now identify individual actions.** Use `X-Folio-Run` for run lookups and `X-Folio-Delivery` for deduplication. Finish pending runs before upgrading when deduplication must span the transition. ([#190](https://github.com/cedricziel/folio/pull/190))

### Added

* Configure OCR, embeddings, and processing through config files or environment variables. Fresh installations honor these defaults; saved settings take precedence. ([#182](https://github.com/cedricziel/folio/pull/182))
* Choose OCRmyPDF for local text recognition from scanned PDFs and images. The container includes its CLI. ([#185](https://github.com/cedricziel/folio/pull/185))
* Export opt-in browser and Apple telemetry through Folio, keeping export credentials on the server. ([#157](https://github.com/cedricziel/folio/pull/157))

### Fixed

* Stop retaining recovery codes and authenticator setup secrets in Turbo's page cache after navigation. ([#187](https://github.com/cedricziel/folio/pull/187))
* Reject malformed or omitted classification IDs without clearing existing metadata. Explicit empty values still clear the selected field. ([#189](https://github.com/cedricziel/folio/pull/189))

[Full changelog](https://github.com/cedricziel/folio/compare/v0.4.0...v0.5.0)

## [0.4.0](https://github.com/cedricziel/folio/compare/v0.3.1...v0.4.0) (2026-11-02)


### Features

* **mcp:** serve archive workflow skills ([5213781](https://github.com/cedricziel/folio/commit/5213781ff8f89aa6a03cd9bf03f75e94939a37c8))
* **mcp:** serve archive workflow skills ([#183](https://github.com/cedricziel/folio/issues/183)) ([7b0caba](https://github.com/cedricziel/folio/commit/7b0cabafd024e19086791edaf7271e9a1ff7ccea))


### Bug Fixes

* **apple:** bound File Provider staging filenames ([24cb80d](https://github.com/cedricziel/folio/commit/24cb80de6a5368be4543a03c329bb47ab340efdf))
* **apple:** derive document status from pipeline stages ([c3119ac](https://github.com/cedricziel/folio/commit/c3119ac0519aa9008ceafbb21ecec6f6b861edeb))
* **apple:** derive document status from pipeline stages ([#158](https://github.com/cedricziel/folio/issues/158)) ([599d81e](https://github.com/cedricziel/folio/commit/599d81e986a94ab18240580a5d13fdce46994e50))
* **apple:** isolate File Provider download handoffs ([5bbcc04](https://github.com/cedricziel/folio/commit/5bbcc04a02ab0d27594420482f76ca50ce4b8eae))
* **apple:** isolate File Provider download handoffs ([#179](https://github.com/cedricziel/folio/issues/179)) ([c809d9f](https://github.com/cedricziel/folio/commit/c809d9f402f89685e8a919c8a0fecd8ae8081b8f))
* **apple:** persist File Provider enumeration snapshots ([dc28896](https://github.com/cedricziel/folio/commit/dc288961135fb67ad241a0f7be3312d37d7ec64a))
* **apple:** persist File Provider enumeration snapshots ([#181](https://github.com/cedricziel/folio/issues/181)) ([1a5dc83](https://github.com/cedricziel/folio/commit/1a5dc83e0ea45cf338b5ff549587f6a31274431f))
* **apple:** report actionable File Provider download errors ([1a46a2c](https://github.com/cedricziel/folio/commit/1a46a2cae092657fc2099b116d9ae0965c43ce5d))
* **apple:** report actionable File Provider download errors ([#184](https://github.com/cedricziel/folio/issues/184)) ([d1cab5a](https://github.com/cedricziel/folio/commit/d1cab5a386c5018aaa7541cfb33bd3cc614416b5))
* **apple:** return uploaded File Provider items before processing ([8f7fde8](https://github.com/cedricziel/folio/commit/8f7fde81c21e571ce14a9873bf0b380f02e0ca97))
* **apple:** return uploaded File Provider items before processing ([#180](https://github.com/cedricziel/folio/issues/180)) ([4abb936](https://github.com/cedricziel/folio/commit/4abb936a7af4f3a6151d0aff98e1327e0c8e3615))

## [0.3.1](https://github.com/cedricziel/folio/compare/v0.3.0...v0.3.1) (2026-11-02)

### Fixed

- Preserve document loading across navigation in the Apple app. ([#112](https://github.com/cedricziel/folio/pull/112))
- Prevent the macOS app from crashing when login completes. ([#114](https://github.com/cedricziel/folio/pull/114))

## [0.3.0](https://github.com/cedricziel/folio/compare/v0.2.0...v0.3.0) (2026-11-02)


### Features

* **auth:** add TOTP two-factor and passkeys ([#80](https://github.com/cedricziel/folio/issues/80)) ([5f2a102](https://github.com/cedricziel/folio/commit/5f2a102d22cc06043f5a74b6c6dd479fc10abbfe))
* **events:** persist events in a transactional outbox ([#86](https://github.com/cedricziel/folio/issues/86)) ([98c455b](https://github.com/cedricziel/folio/commit/98c455b88ea94bc6ab4bd29a0a94efbe92987307))
* **settings:** add an mcp.enabled toggle to /settings ([44c015f](https://github.com/cedricziel/folio/commit/44c015f72b816e8f64cc36a44cd5d98ee978f321))
* **vaults:** add vault isolation and access-policy foundations ([#78](https://github.com/cedricziel/folio/issues/78)) ([9e3d017](https://github.com/cedricziel/folio/commit/9e3d017e03727942a372f6127bca61c83c4855af))
* **web:** show the signed-in user in the sidebar ([#99](https://github.com/cedricziel/folio/issues/99)) ([09462d7](https://github.com/cedricziel/folio/commit/09462d701bcd5a26f36fecdee99f9c7cba241a81))


### Bug Fixes

* **auth:** integrate current crypto and MCP changes ([107bdb8](https://github.com/cedricziel/folio/commit/107bdb8010be44746a3359cb01b8b799fa957ec3))
* **auth:** integrate MFA with current main ([bf204fe](https://github.com/cedricziel/folio/commit/bf204fef451915bf93e225b24d37e939a7031d89))
* **auth:** keep optional passkey autofill quiet ([92b321e](https://github.com/cedricziel/folio/commit/92b321ebad9d7c02dd1066dc004a157c3940b0ae))
* **auth:** make TOTP enrollment updates conditional ([19448d4](https://github.com/cedricziel/folio/commit/19448d4d4b1feadf379ddf7f0422920f3472730b))
* **auth:** report the actual MFA retry delay ([435b4e8](https://github.com/cedricziel/folio/commit/435b4e81f9f1c720ca14618d7d43340a0f1ff1c2))
* **auth:** require fresh proof before passkey enrollment ([5d719c6](https://github.com/cedricziel/folio/commit/5d719c6789389fce955055ddf6bf41d5903880d5))
* **auth:** use current OS random source API ([7dfa39d](https://github.com/cedricziel/folio/commit/7dfa39dc0e02ddec9cc9a61e20e004d55f82c914))
* **auth:** warn when configured secret replaces stored key ([96eeaae](https://github.com/cedricziel/folio/commit/96eeaae0ce68a425409737e2a8d693bf378826d6))
* **config:** redact auth.secret_key in Debug output ([f6a8399](https://github.com/cedricziel/folio/commit/f6a83990da8bfdbf2b6e08668de851e3602783ed))
* **events:** broadcast before running workflows ([4fcd65b](https://github.com/cedricziel/folio/commit/4fcd65b310018b3c540131f3cb4af303ff257271))
* **events:** emit one event per metadata save ([5674108](https://github.com/cedricziel/folio/commit/56741080efd87b0e4ead4f1549aa3d76a26198c6))
* **events:** recover concurrent broadcasts safely ([#109](https://github.com/cedricziel/folio/issues/109)) ([81faf21](https://github.com/cedricziel/folio/commit/81faf211bb3cbee78804c78b28d409507365d0e7))
* **events:** recover overlapping and pending broadcasts ([3734a03](https://github.com/cedricziel/folio/commit/3734a03293fbcd0e5d7ab859cd5fa456f5ac9500))
* **events:** replay events in dispatch order ([9d01b4e](https://github.com/cedricziel/folio/commit/9d01b4e3261f0dfe53e8dab4282056a7923d6be9))
* **infra:** serialize metadata reads with their writes ([4588fd5](https://github.com/cedricziel/folio/commit/4588fd5ef3e63dfc3f9f42e44749fab74a715a50))
* **mcp:** address the MCP review follow-ups ([#107](https://github.com/cedricziel/folio/issues/107)) ([66583c8](https://github.com/cedricziel/folio/commit/66583c808d8b464296d1c79ff120e99c8071b0b3))
* **mcp:** clear historical failures after reprocessing ([929caa9](https://github.com/cedricziel/folio/commit/929caa9111eb453784082df3b17d5a4ac1ec3d0a))
* **mcp:** integrate the transactional outbox ([813c0a2](https://github.com/cedricziel/folio/commit/813c0a27bb5710b63c2a0bf448a26c6b7624327f))
* **mcp:** preserve failures in unrelated processing stages ([ca5cf8f](https://github.com/cedricziel/folio/commit/ca5cf8fa26b353643d291102f60af4032456c5bb))
* **mcp:** preserve failures in unrelated processing stages ([#110](https://github.com/cedricziel/folio/issues/110)) ([1ae557d](https://github.com/cedricziel/folio/commit/1ae557d3f2d40535a18ab170e1664cced272f0bf))
* **vaults:** complete scoped repository integration ([44c2da8](https://github.com/cedricziel/folio/commit/44c2da8ee2470c104367e0ad33d686b6fb965b5a))
* **vaults:** integrate account security page context ([b9dc506](https://github.com/cedricziel/folio/commit/b9dc506ff87b5af0fa4aff414d1bfe5787991ba8))
* **vaults:** integrate durable events and bootstrap access ([9ac88c8](https://github.com/cedricziel/folio/commit/9ac88c81a0fcf53b5ef259e2ce815a6992587be9))
* **vaults:** integrate scoped MCP search ([b52feed](https://github.com/cedricziel/folio/commit/b52feed78b513486b2942aac621cc1728f16c92b))
* **vaults:** pass default vault through existing routes ([8875c7b](https://github.com/cedricziel/folio/commit/8875c7bf7a24a22023cbda18b1910d0785f2ec1c))
* **vaults:** pass vault context to resource rendering ([8702475](https://github.com/cedricziel/folio/commit/87024755cbf12115f5ddfb00c3f74f9676953f0c))
* **vaults:** preserve deletion snapshots across upgrades ([f419678](https://github.com/cedricziel/folio/commit/f419678000524cbb94d77efd3e351178a740c649))
* **vaults:** resolve integration with current main ([c29078e](https://github.com/cedricziel/folio/commit/c29078e60b3d3890ce6c0403cbd542ff082bac19))


### Performance Improvements

* **mcp:** cache the mcp.enabled setting instead of reading it per request ([41f3c2b](https://github.com/cedricziel/folio/commit/41f3c2b3a38d34c6927dd0d0a264d8645881922f))
* **mcp:** search by ids and read the tag table once ([0147fed](https://github.com/cedricziel/folio/commit/0147fed4199df6e9bd966970ac53646c7827c352))

## [0.2.0](https://github.com/cedricziel/folio/compare/v0.1.1...v0.2.0) (2026-11-01)


### Features

* **apple:** link the deployment guide from onboarding ([6b1e5f9](https://github.com/cedricziel/folio/commit/6b1e5f96686af3e97266ca82364ff2c34262fdc3))


### Bug Fixes

* **api:** resolve API and worker backlog issues ([#94](https://github.com/cedricziel/folio/issues/94)) ([f0fc77e](https://github.com/cedricziel/folio/commit/f0fc77e1c1104861ab652cf7020d2cd40213c5af))
* **config:** let the shipped config follow auth for MCP ([70c2337](https://github.com/cedricziel/folio/commit/70c2337d3b6c587532fc24f5b724355fc1e98826))
* **config:** let the shipped config follow auth for MCP ([#96](https://github.com/cedricziel/folio/issues/96)) ([3f90966](https://github.com/cedricziel/folio/commit/3f909660584a87d1f4c51948d0bbf891f369e778))
* **docker:** install ghostscript and allow the PDF coder for the ImageMagick fallback ([18ace36](https://github.com/cedricziel/folio/commit/18ace3657e8fb4f51fc14e9eb4888dee982b2cf2))
* **docker:** make the PDF fallback work in the image ([#98](https://github.com/cedricziel/folio/issues/98)) ([0f5fe7a](https://github.com/cedricziel/folio/commit/0f5fe7a55e2787adbe5581efee523aa58d75b45f))

## [0.1.1](https://github.com/cedricziel/folio/compare/v0.1.0...v0.1.1) (2026-11-01)


### Bug Fixes

* **apple:** take app and build versions from the build settings ([#87](https://github.com/cedricziel/folio/issues/87)) ([42d4819](https://github.com/cedricziel/folio/commit/42d481982e6bc28f2ded0eb525a39c5957864359))

## [0.1.0](https://github.com/cedricziel/folio/compare/v0.1.0-alpha.2...v0.1.0) (2026-11-01)


### Features

* **api:** include tags and filters in the document list API ([#83](https://github.com/cedricziel/folio/issues/83)) ([839115c](https://github.com/cedricziel/folio/commit/839115ceeb2840c4a203bbb4536af98e6276ed3e))
* **api:** include tags and names in documents and filter the list by tag and date ([e29d70d](https://github.com/cedricziel/folio/commit/e29d70dc5fa625e24aa2fdbc9e32ef65fcee1e6e))
* **apple:** read tags from the API and filter documents on the server ([da3a6e3](https://github.com/cedricziel/folio/commit/da3a6e392d854f93427eec5315ae4ff1e70b8867))


### Bug Fixes

* **apple:** declare the File Provider document group ([bcb113b](https://github.com/cedricziel/folio/commit/bcb113b63c0ea48662d4d3dfe5cd1840cd2caacc))
* **apple:** declare the File Provider document group ([#74](https://github.com/cedricziel/folio/issues/74)) ([7253c60](https://github.com/cedricziel/folio/commit/7253c604b1263b4487395a9b2404e4ca51622bd3))
* **apple:** derive build numbers from the UTC build time ([2ce54c5](https://github.com/cedricziel/folio/commit/2ce54c5d9657ef8c859ef7208e4678aa090f39d9))
* **apple:** tell gym the macOS build targets the macosx SDK ([0e0fbfd](https://github.com/cedricziel/folio/commit/0e0fbfdb79161235af694acc06ab6e6020e6ac8b))
* **apple:** tell gym the macOS build targets the macosx SDK ([#75](https://github.com/cedricziel/folio/issues/75)) ([6a90528](https://github.com/cedricziel/folio/commit/6a90528428ba8daaa583a015a5d27678a937e130))
* **apple:** use the commit count as the build number ([52ac2de](https://github.com/cedricziel/folio/commit/52ac2defbe2f3fec60041105bb9f375eefe294ef))
* **apple:** use the commit count as the build number ([#85](https://github.com/cedricziel/folio/issues/85)) ([478a6f5](https://github.com/cedricziel/folio/commit/478a6f5eaae4ecfe1626bd389642061d570c1c80))
* **web:** address the UX review findings ([#77](https://github.com/cedricziel/folio/issues/77)) ([340b3fc](https://github.com/cedricziel/folio/commit/340b3fc81f5f57b3761440f9541dab96b181aeea))
* **web:** neutral generated placeholders, tidy palette dates and list meta ([46157ca](https://github.com/cedricziel/folio/commit/46157caca78d1879bb08db9e915fe85b26e70183))

## [0.1.0-alpha.2](https://github.com/cedricziel/folio/compare/v0.1.0-alpha.1...v0.1.0-alpha.2) (2026-11-01)


### Features

* add a workflow engine with signed webhooks ([#22](https://github.com/cedricziel/folio/issues/22)) ([cf28f10](https://github.com/cedricziel/folio/commit/cf28f10ae532922c4925958944ea6f24fae84902))
* **api:** add /healthz and /readyz probes and a container healthcheck ([#33](https://github.com/cedricziel/folio/issues/33)) ([c145374](https://github.com/cedricziel/folio/commit/c145374fbae9831a780a20ea6023cfc002f38ab4))
* **api:** expose workflows, test deliveries, and runs over the JSON API ([3268765](https://github.com/cedricziel/folio/commit/32687650c1f4d76257fa33e7f4abe3783093eca7))
* **api:** manage ingestion sources in the API and a Sources settings page ([1ab28b8](https://github.com/cedricziel/folio/commit/1ab28b88b9cd73950995718bc4e78573e0a702a8))
* **api:** serve unauthenticated /healthz and /readyz probes ([7574d98](https://github.com/cedricziel/folio/commit/7574d98ac9d079cb3ca93e12e9f15bb69620fe2a))
* **api:** stream downloads and thumbnails from the store and purge objects on delete ([8cd0789](https://github.com/cedricziel/folio/commit/8cd0789579a98f2a4020125167c2a7ebf7d2e7fb))
* **api:** trace and measure HTTP requests with semantic-convention attributes ([61c9371](https://github.com/cedricziel/folio/commit/61c93712f361bcf6bb19f687fca90c984ac05114))
* **app:** add healthcheck subcommand ([80255bc](https://github.com/cedricziel/folio/commit/80255bc2fa898818b5b6df61f838dcea27b7a934))
* **apple:** add App Intents and entities for documents, tags, correspondents and types ([319d1be](https://github.com/cedricziel/folio/commit/319d1be1a9b96b263cf60a49399571723b508b03))
* **apple:** add App Intents for Shortcuts and Siri ([#68](https://github.com/cedricziel/folio/issues/68)) ([7a3e181](https://github.com/cedricziel/folio/commit/7a3e1812168dd29652d642cb018a86bf8b89101e))
* **apple:** add DocumentCatalog for Shortcuts and Siri ([59c346d](https://github.com/cedricziel/folio/commit/59c346d5c6600a6b2ceb4c40e09e08668af2b07e))
* **apple:** add keyboard commands, an inbox scope and intent routing ([bf5ccdf](https://github.com/cedricziel/folio/commit/bf5ccdf166bb49a82c9879985a2e84178ddc9db2))
* **apple:** add removeTag to the Folio service ([1d4366c](https://github.com/cedricziel/folio/commit/1d4366c418c368a7c41b152c2627525f9ceb2ef1))
* **apple:** add share and file provider extensions ([#30](https://github.com/cedricziel/folio/issues/30)) ([a43413f](https://github.com/cedricziel/folio/commit/a43413fe918aec6e713ccdc6133b78f540b294f6))
* **apple:** add share and file provider extensions for iOS and macOS ([8b2914e](https://github.com/cedricziel/folio/commit/8b2914e9c15594c31e2c49c12e88bcb53593a5f6))
* **apple:** declare Siri phrases and index viewed documents in Spotlight ([de1a88c](https://github.com/cedricziel/folio/commit/de1a88c095e666912f61df15f8df7dd7d4685d9a))
* **apple:** scaffold native iOS and macOS app ([#25](https://github.com/cedricziel/folio/issues/25)) ([fb7c124](https://github.com/cedricziel/folio/commit/fb7c1247d488607866907a53133c018ff955ea2c))
* **apple:** sign in with OAuth 2.1 and PKCE ([3e3fa20](https://github.com/cedricziel/folio/commit/3e3fa20e4c068dc4e4b24113f20d3a721065776f))
* **apple:** sign in with OAuth 2.1 and PKCE ([#53](https://github.com/cedricziel/folio/issues/53)) ([3051cb4](https://github.com/cedricziel/folio/commit/3051cb4207877531703dde07e4612c4fe898e53c))
* **app:** serve MCP over HTTP at /mcp and stdio via 'mcp --stdio' ([0616288](https://github.com/cedricziel/folio/commit/0616288b0a96bfc1bf851cf88b267b4007d72db9))
* **app:** start telemetry on boot and flush it on shutdown ([2909ce4](https://github.com/cedricziel/folio/commit/2909ce4eca970cca85ce80edcec3fb03f3585b2a))
* **app:** wire ingestion, record sources on uploads, and show them on documents ([c78f706](https://github.com/cedricziel/folio/commit/c78f7061cba30e89f30ff273c0f8d60aee049640))
* **app:** wire the workflow engine, runner, and public URL setting ([d04cff8](https://github.com/cedricziel/folio/commit/d04cff87e37ddc02500ad3734feee4b336ef9e5e))
* **auth:** add forward auth login with trusted proxies and groups ([#52](https://github.com/cedricziel/folio/issues/52)) ([e161a3d](https://github.com/cedricziel/folio/commit/e161a3dafc39de2373aee2b97ed71b96835af8e3))
* **auth:** add forward auth tests, Postgres store and docs ([3dd5b43](https://github.com/cedricziel/folio/commit/3dd5b4324b75bf5186a47ff475b5ab6a57b42b9c))
* **auth:** add forward auth, external users and groups ([59fa724](https://github.com/cedricziel/folio/commit/59fa724db70bc6b5714c07ddc4f03d1c5628d9c1))
* **auth:** add the OpenID Connect relying party ([963cd59](https://github.com/cedricziel/folio/commit/963cd59cc6148fdf3071ef15d780fcd4666c9de1))
* **auth:** keep the OIDC ID token with the session ([fc3fe8b](https://github.com/cedricziel/folio/commit/fc3fe8b8633dcc9aa24fac82eebb2c993ab3fabd))
* **auth:** link forward-auth accounts by a stable id header when configured ([d3f2584](https://github.com/cedricziel/folio/commit/d3f258470e269ac366a75451dcf77b2bd3bcbba9))
* **auth:** optionally require a shared secret from the forward-auth proxy ([c69a83f](https://github.com/cedricziel/folio/commit/c69a83f2810c309136096b9a25dbaf1dd638cdf0))
* **auth:** require login for the web UI and API (auth.required) ([5100c70](https://github.com/cedricziel/folio/commit/5100c70b32adb2b4fcae5fbd491453a42fc7ff7d))
* **auth:** sign in with OpenID Connect ([12fddec](https://github.com/cedricziel/folio/commit/12fddec1693a25b25456c2ab1c2604dea30b3476))
* **auth:** sign in with OpenID Connect ([#54](https://github.com/cedricziel/folio/issues/54)) ([aecda2e](https://github.com/cedricziel/folio/commit/aecda2e6b9d1af02f952622e5f9ea0673aef0f58))
* **auth:** store in-flight OIDC logins ([93155a8](https://github.com/cedricziel/folio/commit/93155a8bbfbbae21d152823820e9042e26af3aa6))
* **config:** add auth.oidc settings ([1d641c9](https://github.com/cedricziel/folio/commit/1d641c90384b1eb47f161d66c06750f44abf64ad))
* **core:** add library service for assistant-facing archive operations ([f5ea9c5](https://github.com/cedricziel/folio/commit/f5ea9c5bb862360db100ae6689c19252e7705b28))
* **core:** add the DocumentStore port and object key layout ([3f49649](https://github.com/cedricziel/folio/commit/3f49649545ae8afef1bca7f36f1d4083348c1da1))
* **core:** add the workflow engine ([1c2901a](https://github.com/cedricziel/folio/commit/1c2901aa36d87f38654fe0dee071652482982eb3))
* **core:** add workflow domain model and ports ([90b8ac6](https://github.com/cedricziel/folio/commit/90b8ac6fec0bf8b65ad25e9613410427033dba67))
* **core:** emit document lifecycle events from the repository ([5231368](https://github.com/cedricziel/folio/commit/5231368e9de310415d6988ff02979c8b150ae305))
* **core:** model document sources, ingestion sources, and the import pipeline ([9fb897b](https://github.com/cedricziel/folio/commit/9fb897bb199fd2bd2ddad5f2ca56a57eada3459f))
* **db:** add PostgreSQL adapters selected by the database URL scheme ([64d9d6b](https://github.com/cedricziel/folio/commit/64d9d6beef6216c4a0fc67730bb0f525f2ed4130))
* **db:** support PostgreSQL alongside SQLite ([#38](https://github.com/cedricziel/folio/issues/38)) ([c9c95c6](https://github.com/cedricziel/folio/commit/c9c95c68c8ecc43b184a131a0370c87e0365a3cf))
* **e2e:** add Playwright suite for the document lifecycle ([ee096b8](https://github.com/cedricziel/folio/commit/ee096b850b3a60c0ba4995fb1eb6c32189adc5c4))
* **e2e:** add the verify skill and a Playwright e2e suite ([#36](https://github.com/cedricziel/folio/issues/36)) ([1144d24](https://github.com/cedricziel/folio/commit/1144d242344b4726e85a0a7d3a59867e572371e7))
* **infra:** add HealthCheck port and database adapter ([cdfb54b](https://github.com/cedricziel/folio/commit/cdfb54b772928cc36e5fb487fb2224478064acde))
* **infra:** add mcp configuration section ([19f74ee](https://github.com/cedricziel/folio/commit/19f74ee916916345fb7dd195f3d2975555187cc5))
* **infra:** honour tag, text, and date filters when listing documents ([f526ae6](https://github.com/cedricziel/folio/commit/f526ae6096517834669348ec863307d49be45c9c))
* **infra:** import attachments from IMAP mailboxes ([40c54f6](https://github.com/cedricziel/folio/commit/40c54f6a5481c42c8a76ad645b15b7f7d7454bf7))
* **infra:** import files from a watched consume folder ([dd21da5](https://github.com/cedricziel/folio/commit/dd21da594a2067a3a17663d79d64640b3693531f))
* **infra:** load ingestion sources from configuration and build adapters ([6837c3f](https://github.com/cedricziel/folio/commit/6837c3fdf280a12b636eb6d275aa13e3d78202b5))
* **infra:** persist document sources and the ingestion ledger ([9e6e00d](https://github.com/cedricziel/folio/commit/9e6e00d72381b9a91f38b1b85290266023550aa6))
* **infra:** persist workflows and deliver signed webhooks via the job queue ([9393bf0](https://github.com/cedricziel/folio/commit/9393bf0fa53d85611e26fdd5e73e513d3cd66e46))
* **infra:** require every search term to match a document ([c44ec02](https://github.com/cedricziel/folio/commit/c44ec0204874c00f4621cc62765d9de75ed0b729))
* **infra:** trace jobs, AI provider calls and storage operations ([c286dd7](https://github.com/cedricziel/folio/commit/c286dd7a545d06bea5adc77150b6c283002bb0a1))
* ingest documents from a consume folder and IMAP mailboxes ([#45](https://github.com/cedricziel/folio/issues/45)) ([85171c0](https://github.com/cedricziel/folio/commit/85171c0cc453b31939468f262bcca96869b5c182))
* **mcp:** add MCP server crate with archive tools and resources ([84c75ea](https://github.com/cedricziel/folio/commit/84c75ead11c778f6aef6ee0ffdbce9a3c30ab322))
* **mcp:** add MCP server for AI assistants ([#31](https://github.com/cedricziel/folio/issues/31)) ([7c158bf](https://github.com/cedricziel/folio/commit/7c158bf18a9d4642f6f660b490633b0fafc8ded9))
* **pipeline:** pluggable OCR and embedding providers ([#35](https://github.com/cedricziel/folio/issues/35)) ([324e8df](https://github.com/cedricziel/folio/commit/324e8df585a40675fce3e4049fdfdf3f2bf4e4ba))
* **skill:** add a screen tour with screenshots and axe checks to verify ([e2a97a6](https://github.com/cedricziel/folio/commit/e2a97a61b6864de9b1313c6b192c3c9cbe873645))
* **storage:** add a storage port with local and S3 adapters ([#43](https://github.com/cedricziel/folio/issues/43)) ([fba5375](https://github.com/cedricziel/folio/commit/fba5375b06e7367391a7e512ab3234da2e625750))
* **storage:** add local and S3 document stores over object_store ([51d6016](https://github.com/cedricziel/folio/commit/51d601690b10896f590a00a9cd5a7f9880e57102))
* **telemetry:** export traces, metrics and logs with OpenTelemetry ([#47](https://github.com/cedricziel/folio/issues/47)) ([509513e](https://github.com/cedricziel/folio/commit/509513e65c68375824924e2841174f8780bb6ef7))
* **telemetry:** initialise OTLP export from the standard OTEL variables ([bcda36a](https://github.com/cedricziel/folio/commit/bcda36af567ffe23c424b3464a396d429badb78b))
* **verify:** add scripts to run an ephemeral Folio instance ([a3cce68](https://github.com/cedricziel/folio/commit/a3cce68bbd09e025cb33aa342cf6e94149644007))
* **web:** add a workflows page with create, edit, test, and run history ([211af76](https://github.com/cedricziel/folio/commit/211af769cf15f95596a251c4d697e3fbf4838274))
* **web:** add keyboard shortcuts ([#67](https://github.com/cedricziel/folio/issues/67)) ([189e1b8](https://github.com/cedricziel/folio/commit/189e1b8d32b002bff4a2c84c84236213fc6ceb83))
* **web:** add keyboard shortcuts with a help dialog and in-page delete confirm ([98ea0c3](https://github.com/cedricziel/folio/commit/98ea0c37f8a834c9e738eeab23473670e8b1ee52))
* **web:** reskin the web UI with the Folio design system and ship it as a PWA ([#34](https://github.com/cedricziel/folio/issues/34)) ([988022e](https://github.com/cedricziel/folio/commit/988022ec0fd64942912ada11e6bb84d375a82e8e))


### Bug Fixes

* **api:** mask webhook header values, hide URL credentials, and use axum 0.8 routes ([d685906](https://github.com/cedricziel/folio/commit/d685906348efcea45831fbcbfd34415b2c42404f))
* **api:** serve uploaded files with nosniff, a sandbox CSP and attachment for active types ([7e76361](https://github.com/cedricziel/folio/commit/7e7636132ea799deb2ae2642d12f598e7c491229)), closes [#48](https://github.com/cedricziel/folio/issues/48)
* **apple:** add the app icon and export macOS builds as pkg ([#69](https://github.com/cedricziel/folio/issues/69)) ([92aa948](https://github.com/cedricziel/folio/commit/92aa948a07b06cbb733b5754ae40ddbf251afb47))
* **apple:** add the app icon generated from the design mark ([171d1dd](https://github.com/cedricziel/folio/commit/171d1dd9c0682dd445a9ff715cdc4312e3f9d361))
* **apple:** export macOS builds as a sandboxed installer pkg ([fcf67dd](https://github.com/cedricziel/folio/commit/fcf67ddd2f0f29090305f5a1e9ab4d391bdccfdf))
* **apple:** harden downloads, search state, uploads and file picking ([93bcd6a](https://github.com/cedricziel/folio/commit/93bcd6af0fad3ebfd18b2edd1551746c7fddc94a))
* **apple:** support all iPad orientations for multitasking ([0952631](https://github.com/cedricziel/folio/commit/09526319f7c9bc2799028fe99f90931e643801b7))
* **apple:** support all iPad orientations for multitasking ([#57](https://github.com/cedricziel/folio/issues/57)) ([429b777](https://github.com/cedricziel/folio/commit/429b7778822f0cb7982bc55a2908d467fe5c3fd9))
* **apple:** wire AppIcon into the Folio target ([b540226](https://github.com/cedricziel/folio/commit/b54022694dc4bbfcc6121a1181edbca41f398db7))
* **auth:** add the RFC 9207 iss parameter to authorization responses ([2e68e9c](https://github.com/cedricziel/folio/commit/2e68e9c16ca505acd121897b67d7075f94b085f9))
* **auth:** bind CSRF tokens to the session with an HMAC and use __Host- cookies over https ([8b9ff88](https://github.com/cedricziel/folio/commit/8b9ff888be251c1b27edc061b18bc542351c8d9f))
* **auth:** bound dynamic client registration and purge unused clients periodically ([f10ef6c](https://github.com/cedricziel/folio/commit/f10ef6c7b7c952c8a79b2ea80b8d61f8c47f6efd))
* **auth:** configurable group separator and a warning for catch-all trusted proxies ([44c1a40](https://github.com/cedricziel/folio/commit/44c1a4073d7fdb1c85a60d94fb5db5f12b7d6371))
* **auth:** create the first external user atomically ([bc3a096](https://github.com/cedricziel/folio/commit/bc3a0968f269855bafa0c753f2fa0c1aea495236))
* **auth:** delete the previous session on login ([000d2ff](https://github.com/cedricziel/folio/commit/000d2ff7c5ceeb0310b1c6e0ca3654d13ec45370))
* **auth:** derive issuer and cookie flags from public_url only and require it when exposed ([e203120](https://github.com/cedricziel/folio/commit/e20312023b9e2a89289c8ecc305054a635bb93e9))
* **auth:** enforce documents:read/write scopes on bearer tokens for /api ([4bd7850](https://github.com/cedricziel/folio/commit/4bd78502dc7cc449f90285486a09c0d73bc29006))
* **auth:** forbid framing and caching of login, setup and consent pages ([004984c](https://github.com/cedricziel/folio/commit/004984cf254addd9e30e5599c589d2f42136289c))
* **auth:** group tokens into grants; revoke the family on refresh reuse and revoke ([dc4dd1d](https://github.com/cedricziel/folio/commit/dc4dd1dfe50e40a104ef3cbc5667330d64c4157d))
* **auth:** harden OIDC token validation and discovery ([8612301](https://github.com/cedricziel/folio/commit/86123019f8dc0a3fa8c46eabde49ef5f0718efdb))
* **auth:** harden the authorization server and login ([#55](https://github.com/cedricziel/folio/issues/55)) ([19312ba](https://github.com/cedricziel/folio/commit/19312bab207e28442e907d69253476cf9afc1a75)), closes [#48](https://github.com/cedricziel/folio/issues/48)
* **auth:** ignore forward headers on OAuth, MCP and bearer-token requests ([97e6215](https://github.com/cedricziel/folio/commit/97e621598ef6a91f266aab01cd5fb7078f25a3e6))
* **auth:** ignore forwarded groups unless configured and add role-based admin ([d50b486](https://github.com/cedricziel/folio/commit/d50b48614fce39b2bcb8376ae9244112001de555))
* **auth:** keep a permanent owner that group and role mappings cannot demote ([26ab192](https://github.com/cedricziel/folio/commit/26ab192021861c202d8e34b9bad3d4634bc2e658))
* **auth:** keep PWA assets public, refuse anonymous shares, serve /api resource metadata ([72bb5a9](https://github.com/cedricziel/folio/commit/72bb5a99162015f87c1372bd063db18c06bec407))
* **auth:** let concurrent first forward-auth requests share one account ([03451f7](https://github.com/cedricziel/folio/commit/03451f7941f5e6d65935b5ba120af3ee3752fd60))
* **auth:** make first-user creation atomic on PostgreSQL ([67053e1](https://github.com/cedricziel/folio/commit/67053e17207907762a6b15b50f2445f6bd9a1539))
* **auth:** move the owner migration to 20261101220000 to avoid version clashes ([4b77b2b](https://github.com/cedricziel/folio/commit/4b77b2b9ba977c8cac1bb4165d43e09e99b61b5e))
* **auth:** purge expired auth data in the background ([44e3fef](https://github.com/cedricziel/folio/commit/44e3feff03ce99a9443d5f39019aad291cab84fc))
* **auth:** rate limit OIDC starts, bind the cookie to the host and hint the provider at logout ([469533f](https://github.com/cedricziel/folio/commit/469533f9afb26a3b28efc07976338d9346975c34))
* **auth:** reject cross-origin writes that rely on the session cookie ([926a882](https://github.com/cedricziel/folio/commit/926a88295621d81bb47c1699d372c8b88e5b3bed))
* **auth:** reject requests that repeat a forward identity header ([cd3ea1a](https://github.com/cedricziel/folio/commit/cd3ea1a56a99171a71d8cf34fe0828d574289b8b))
* **auth:** renumber the OIDC migrations after main's ingestion ([0693a2d](https://github.com/cedricziel/folio/commit/0693a2d534c4183371541d4aa550ba6eb07ec41a))
* **auth:** renumber the owner migration past main's ingestion one and satisfy newer clippy ([7421ed2](https://github.com/cedricziel/folio/commit/7421ed27cd2c4358bffa61053783ee47593ddaf3))
* **auth:** require a one-time setup token while no user exists ([25ac050](https://github.com/cedricziel/folio/commit/25ac05011b8edc24bd29e347eb252b32f46842c0))
* **auth:** serialise external group syncs per user ([0f27067](https://github.com/cedricziel/folio/commit/0f27067d82f36d64640cf0f81275d6174e8bc6db))
* **auth:** shorter, reused and revocable forward-auth sessions ([293fe2f](https://github.com/cedricziel/folio/commit/293fe2f0df222d94e5baaf1bdb67d8c5ee56bee4))
* **auth:** strip every forward identity header from untrusted peers ([5271c46](https://github.com/cedricziel/folio/commit/5271c4638f14a077bb4067ccc8ea4e03fb2d1a0c))
* **auth:** throttle failed logins, cap password length and concurrent hashing ([712c6b8](https://github.com/cedricziel/folio/commit/712c6b8064b6d19562a6bec0904cc245cb643090))
* **config:** read nested settings from double-underscore env vars ([685f920](https://github.com/cedricziel/folio/commit/685f9200aa32036adcc9d58cce68915ffe0d0b13))
* **config:** read nested settings from double-underscore env vars ([#26](https://github.com/cedricziel/folio/issues/26)) ([b6f24ab](https://github.com/cedricziel/folio/commit/b6f24abcd989934ac719ba2242aae1fb57afa774))
* **db:** drop duplicate health module declaration after rebase ([551cf93](https://github.com/cedricziel/folio/commit/551cf9320e9f6b3fcf64cec37e69fc862e55d5a7))
* **db:** give the OAuth grants migration a unique version ([72c1285](https://github.com/cedricziel/folio/commit/72c1285e437f9a8cc0fa96fb7666878daad9f557))
* **infra:** dedupe once-per-document runs with a unique index ([b7488dd](https://github.com/cedricziel/folio/commit/b7488dd43da2f920e3404f7524d0a720e1735415))
* **mcp:** disable the endpoint by default and warn when it is enabled ([cf44377](https://github.com/cedricziel/folio/commit/cf44377a0659f9d696f8b6e5854f0b65b3a0f8c0))
* **mcp:** rank exact matches first, validate tag names before writing, and report server identity ([8502fe2](https://github.com/cedricziel/folio/commit/8502fe2716c5596cd877dfacf2f78076307a326f))
* **pipeline:** adapt to sqlx 0.9 and clippy, simplify test outcome template ([02a3687](https://github.com/cedricziel/folio/commit/02a36875f8978d0b40e2e41163e35d6b86ca7476))
* **search:** fall back to text matching and drive the badge from stage status ([21a80da](https://github.com/cedricziel/folio/commit/21a80da594993fabbe7fa483265d872e8e7b2ef3))
* **skill:** poll page_count and export VERIFY_DIR ([034c89c](https://github.com/cedricziel/folio/commit/034c89c47277e15164ddd528d350b64e89908e53))
* **web:** call macros with the askama 0.16 syntax ([4be80a8](https://github.com/cedricziel/folio/commit/4be80a8f1dd3344872b29309cd9b1db90ff4e1f2))

## [0.1.0-alpha.1](https://github.com/cedricziel/folio/compare/v0.1.0...v0.1.0-alpha.1) (2026-11-01)


### Features

* add background job queue and document thumbnails ([697dba9](https://github.com/cedricziel/folio/commit/697dba917386ee01ece30ed67f1898b9f7fae8f5))
* add command palette with ⌘K shortcut ([b47d570](https://github.com/cedricziel/folio/commit/b47d570aec6099ed47761cb66768cf70d928ec9d))
* add database-stored settings with AI configuration UI ([32358d5](https://github.com/cedricziel/folio/commit/32358d57f041994edc18849e04e1627e7e312ec8))
* add document metadata management with correspondents, types, and tags ([492d4bc](https://github.com/cedricziel/folio/commit/492d4bc7fbc33058cb27a9b026c3d4bc9a7f6d4f))
* add drag-and-drop upload and livereload support ([8b00a8b](https://github.com/cedricziel/folio/commit/8b00a8b8760c611f2dbdbcf1aed25e0718b7f49f))
* add dynamic AI services that read from database settings ([c88e2ee](https://github.com/cedricziel/folio/commit/c88e2ee13eff40ee37d7f6edec5e5f9842798900))
* add full-page document view with editorial design ([2b661c5](https://github.com/cedricziel/folio/commit/2b661c5830d825e6b6ab8f1089cfcd730cda11bc))
* add live search with debouncing ([8286d79](https://github.com/cedricziel/folio/commit/8286d79145d070e2f3b9926263514300e7a4346e))
* add multi-page document viewing and reprocess button ([ac7f1f3](https://github.com/cedricziel/folio/commit/ac7f1f3991eb53aa613bffe583624de3e364c667))
* add SSE for real-time document updates ([85fa8f4](https://github.com/cedricziel/folio/commit/85fa8f4c93bf8b42ce9493a712d605f9d5f72c63))
* **api:** publish OpenAPI spec for the JSON API ([6dca93c](https://github.com/cedricziel/folio/commit/6dca93cc3aefa409f6de4d4bc7e7ba948e9ce49a))
* bootstrap folio document management system ([2ee8ab9](https://github.com/cedricziel/folio/commit/2ee8ab97e752f116464e277c2d565c68b12eafbf))
* **core:** derive ToSchema on API-facing types behind an openapi feature ([892660b](https://github.com/cedricziel/folio/commit/892660b84387038db1235025219aba6708428ec3))
* **design:** add Folio design tokens and component styles ([aadebd4](https://github.com/cedricziel/folio/commit/aadebd406dc5ad8a116050dc2d1915fb8cd9533a))
* improve UX with layout redesign and self-repair mechanism ([7f170a7](https://github.com/cedricziel/folio/commit/7f170a737847b41018803ee43b1b8430cf498010))
* increase PDF OCR resolution to 300 DPI for better text extraction ([9609588](https://github.com/cedricziel/folio/commit/9609588d0f2a096970c5d5d4e970d5ac933ca05c))


### Bug Fixes

* **api:** clear tag color when color is null ([237260b](https://github.com/cedricziel/folio/commit/237260b0a052d080ad647cd14aea71ad2fcc109e))
* **api:** point the spec drift message at the right command ([c9448ba](https://github.com/cedricziel/folio/commit/c9448ba7c2aa09eebcc9de93ff8e6040f189123a))
* **api:** use axum 0.7 path syntax for id routes ([f2d50d2](https://github.com/cedricziel/folio/commit/f2d50d25e8914d9bbfade60e6f5b3a66bb02c40c))
* **api:** use axum 0.8 path syntax for id routes ([5f5883a](https://github.com/cedricziel/folio/commit/5f5883ace2ca5b66a42f9eb9b97087a0b7cf0942))
* convert PDFs to images before OCR ([a99298c](https://github.com/cedricziel/folio/commit/a99298c1de51d58464f59ad1c3fd6074d97473af))
* handle pdftoppm zero-padded output filenames ([cd9065f](https://github.com/cedricziel/folio/commit/cd9065fb0b4d933f4ee04bdb48717491487bdf0e))
