# Changelog

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
