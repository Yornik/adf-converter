# Changelog

## [0.4.0](https://github.com/Yornik/adf-converter/compare/v0.3.0...v0.4.0) (2026-10-28)


### Features

* adf:attachment links for attached files (mediaGroup, mediaInline) ([#21](https://github.com/Yornik/adf-converter/issues/21)) ([d6e0cf2](https://github.com/Yornik/adf-converter/commit/d6e0cf2a6f7a27fd8b737dd0a55d550ae874702c))


### Documentation

* name the package in the cargo install command ([#19](https://github.com/Yornik/adf-converter/issues/19)) ([95b07cc](https://github.com/Yornik/adf-converter/commit/95b07cc684124757bc9c131d61b20b1e27cf2e97))

## [0.3.0](https://github.com/Yornik/adf-converter/compare/v0.2.0...v0.3.0) (2026-10-27)


### ⚠ BREAKING CHANGES

* csv and tsv code fences become tables ([#16](https://github.com/Yornik/adf-converter/issues/16))

### Features

* csv and tsv code fences become tables ([#16](https://github.com/Yornik/adf-converter/issues/16)) ([51dcaae](https://github.com/Yornik/adf-converter/commit/51dcaae05199d7622eeb5e2ee1402df6a01cd588))
* include a CSV file in Markdown with adf:csv under --include-root ([#17](https://github.com/Yornik/adf-converter/issues/17)) ([b92e9ac](https://github.com/Yornik/adf-converter/commit/b92e9ac1a82f7ebbc5d5e41448160acedac8af15))
* native CSV/TSV tables (--from csv, --to csv) ([#15](https://github.com/Yornik/adf-converter/issues/15)) ([8acbd75](https://github.com/Yornik/adf-converter/commit/8acbd75c9186c4985569fb6eee19673945932b91))

## [0.2.0](https://github.com/Yornik/adf-converter/compare/v0.1.0...v0.2.0) (2026-10-27)


### Features

* file media (adf:media images) for Jira and Confluence attachments ([#11](https://github.com/Yornik/adf-converter/issues/11)) ([d5d4299](https://github.com/Yornik/adf-converter/commit/d5d429903c0b2ae87c717c98863ededd56d4916b))


### Documentation

* **skill:** tables, CSV and spreadsheets for the Agent Skill ([#12](https://github.com/Yornik/adf-converter/issues/12)) ([da03a4a](https://github.com/Yornik/adf-converter/commit/da03a4a7e8236d3e54049607227557b66381032e))

## 0.1.0 (2026-10-20)


### Features

* add the markdown-to-adf agent skill ([77f8449](https://github.com/Yornik/adf-converter/commit/77f84497ece5ae9e68ef1fd5245b28fe83ff4bc3))
* cap Markdown input at 1 MiB and emphasis delimiters per paragraph ([e450b92](https://github.com/Yornik/adf-converter/commit/e450b921563b2170268471cc06ce4e75e755a725))
* **catalog:** feature catalog behind capabilities and examples ([100ef59](https://github.com/Yornik/adf-converter/commit/100ef591af5bde66007cf20eb3e4a94229e9636d))
* **cli:** adf command line with JSON diagnostics and atomic output ([e7aafe6](https://github.com/Yornik/adf-converter/commit/e7aafe6b1b20ab0012d279cab77b0272ff082a14))
* **envelope:** wrap and unwrap Jira and Confluence API envelopes ([8d8f4a9](https://github.com/Yornik/adf-converter/commit/8d8f4a9d44cef906a1891f071e47631149e038f0))
* friendlier links, cards and headings in plain Markdown ([57880ba](https://github.com/Yornik/adf-converter/commit/57880badfcef5a7c2741dc5c25be339235fe1394))
* initial Rust implementation of the adf converter ([bfb4308](https://github.com/Yornik/adf-converter/commit/bfb430847b6378eeb2fe997828aa690787571abd))
* lenient ADF reader for ADF to Markdown ([63d025d](https://github.com/Yornik/adf-converter/commit/63d025dc7e8afe3b9cfcac998641e8c3750d90e6))
* **md:** add adf: link extensions and raw ADF validation ([95c073e](https://github.com/Yornik/adf-converter/commit/95c073e8033546492acfc91a0e45e8a25cb36a12))
* **md:** check raw ADF blocks against the target rules ([326142f](https://github.com/Yornik/adf-converter/commit/326142fb377c43b1d84e23865b37a402194c3a2f))
* **md:** convert Markdown to ADF with container rules ([7b73c4a](https://github.com/Yornik/adf-converter/commit/7b73c4a072528256b8443be2f16d474044771b60))
* **md:** normalise Markdown input before parsing ([091574c](https://github.com/Yornik/adf-converter/commit/091574c0a97d19da39939db55e8c2d530ff670c0))
* **md:** tokenize raw HTML and decode entities ([931d1ac](https://github.com/Yornik/adf-converter/commit/931d1acda699c8af05189ac62ad60f494ed10832))
* render ADF as Markdown in roundtrip and plain flavours ([3f6f663](https://github.com/Yornik/adf-converter/commit/3f6f6631095b97db8dc8d9735dcf1ca68e915515))
* **validate:** schema, target and invariant validation with JSON Pointers ([008a664](https://github.com/Yornik/adf-converter/commit/008a664f0647dd24e55d7aad2b4088902fec720a))


### Bug Fixes

* **cli:** clearer usage hints and --strict diagnostics ([e194a96](https://github.com/Yornik/adf-converter/commit/e194a96ca8caeec5d9eab15c65c090f81aa1b49a))
* **cli:** fence examples longer than the code they contain ([6e5f8d5](https://github.com/Yornik/adf-converter/commit/6e5f8d51f157f7f2108a44a49d500d560f508167))
* **cli:** give both nesting limits for E_INPUT_TOO_DEEP ([f023425](https://github.com/Yornik/adf-converter/commit/f02342506ca12261ec6eb10f83cc2e35bd55c37b))
* **cli:** keep `adf help`, and pass through self-check depth errors ([7149334](https://github.com/Yornik/adf-converter/commit/714933494209e63bcb5d80b06816429d18958e23))
* **cli:** name the binary adf in usage lines on every platform ([9670a23](https://github.com/Yornik/adf-converter/commit/9670a23ca6ec678c1112ab59a6d97ef5e81391aa))
* **cli:** name the missing argument in E_USAGE ([f8f8c4d](https://github.com/Yornik/adf-converter/commit/f8f8c4d59e337895136d87aa9dc82f9bfff73ddf))
* **cli:** reject --field where it selects nothing ([864deac](https://github.com/Yornik/adf-converter/commit/864deac4f78ed385f9cf1790d7f13b93ef41d1cf))
* **cli:** resolve ADF_ERROR_FORMAT in one place ([3d3fd9b](https://github.com/Yornik/adf-converter/commit/3d3fd9bb197f2a2f90e7a44e58a60ff1214275ae))
* **cli:** write -o special files in place and keep the file mode ([65784aa](https://github.com/Yornik/adf-converter/commit/65784aabe1859b2c17d1c5e14da790ce2e586ac3))
* front matter comments, CRLF in ADF input, one warning-summary format ([2807f05](https://github.com/Yornik/adf-converter/commit/2807f05d9905a40b9c2278195d85197ee4830ef2))
* integrate streams and unify ADF nesting limits ([0e18058](https://github.com/Yornik/adf-converter/commit/0e180584ba8a88ed6513399561e2e7855142d60b))
* **md:** avoid a pulldown-cmark panic on empty tight paragraphs ([c676236](https://github.com/Yornik/adf-converter/commit/c67623601c18d352082794ca5814aa332396731b))
* **md:** bound link-label memory and fix lossy Markdown edge cases ([f377dce](https://github.com/Yornik/adf-converter/commit/f377dce842583ceb1eb456b2c4206441de908124))
* **md:** bound Markdown to ADF output at 64 MiB ([c5dcc4f](https://github.com/Yornik/adf-converter/commit/c5dcc4fad4f48bc2d88ac95f4485036ac393c389))
* **md:** cap the internal nesting depth at 100 levels ([c26c46b](https://github.com/Yornik/adf-converter/commit/c26c46b89b3214f0f765fdd8bf578c3f416a2fc9))
* **md:** keep adf:status labels verbatim ([28563d2](https://github.com/Yornik/adf-converter/commit/28563d283b618c33f48bb2164ac1cff24ee1289b))
* **md:** limit table cells to stop GFM row padding from exhausting memory ([0eea24f](https://github.com/Yornik/adf-converter/commit/0eea24ff4c5143a1a32e376f94cdbcd5d3a83ea2))
* **md:** merge code text by its output marks and trim image alt last ([48bdd5e](https://github.com/Yornik/adf-converter/commit/48bdd5ecbae2d606f69810e916f7812049108ccb))
* **md:** report conversions of blocks hoisted from task items ([c17f07d](https://github.com/Yornik/adf-converter/commit/c17f07dfa3407b8b2cad8f2330005795a29a0234))
* **md:** report image titles that --images external drops ([d65542a](https://github.com/Yornik/adf-converter/commit/d65542ae6ff86fbaabc84a7b6e59469f98a8a2c2))
* **md:** validate raw adf blocks in linear time and memory ([014fa68](https://github.com/Yornik/adf-converter/commit/014fa68d6b70b6d1166bd91ab403e3b233abb2f2))
* never leave a dangling newline after empty nested task lists ([8e8d367](https://github.com/Yornik/adf-converter/commit/8e8d36711304981580405a06d394360125018727))
* **release:** let release-please create CHANGELOG.md ([6a205ed](https://github.com/Yornik/adf-converter/commit/6a205edc6c89d8ce2220a3d6faab2a053075e309))
* **to-md:** bound memory, output and stack use on untrusted ADF ([7d4994d](https://github.com/Yornik/adf-converter/commit/7d4994d0e6920a4c2ba9723b85fb3253fe186666))
* **validate:** accept editor list items and annotations, check card data URLs ([ee8aee0](https://github.com/Yornik/adf-converter/commit/ee8aee0fa0a8dbfe9d8870101ee417efbceeff6c))
* **validate:** bound validation memory and list at most 100 diagnostics per code ([eaf9761](https://github.com/Yornik/adf-converter/commit/eaf9761992e4d0b07d408a5c8f5d3fa7bea2ee2b))
* **validate:** plural wording for unknown keys and empty node types ([1807d6c](https://github.com/Yornik/adf-converter/commit/1807d6c24d91c3f8406b150b27d2ecd48d344020))


### Performance Improvements

* **md:** keep autolinking and HTML tag closing linear ([dabc717](https://github.com/Yornik/adf-converter/commit/dabc717b442df579bcf697d76673d0d96b330dbd))


### Documentation

* add AGENTS.md and CLAUDE.md for coding agents ([51c3242](https://github.com/Yornik/adf-converter/commit/51c324246a28b3151de470934d436abb906e231a))
* add contributing guide, security policy, changelog and notices ([7af84a1](https://github.com/Yornik/adf-converter/commit/7af84a16fb5c6de430702e922424dea3eeb1fbb1))
* add envelope reference and posting recipes ([61c2933](https://github.com/Yornik/adf-converter/commit/61c29339f34de06c9d433ebc37c0962c3d2bb307))
* add maintainer guide for repository settings and releases ([6d21109](https://github.com/Yornik/adf-converter/commit/6d21109c80b02015ba7c6a3eadc412e2976f2f79))
* **contract:** document validate --from auto and correct examples and warning sections ([6eb1bef](https://github.com/Yornik/adf-converter/commit/6eb1bef8129478a2e77b404ca62cf925e37e3b4c))
* correct release types, CI checks, release environment and security contact ([c674fb4](https://github.com/Yornik/adf-converter/commit/c674fb4521da9ec60aac869a8e4832c798012225))
* document ADF -&gt; Markdown output limits and plain-flavour changes ([216b7c6](https://github.com/Yornik/adf-converter/commit/216b7c699cdf27ca6b61b3189933c845f6d89783))
* document the Markdown fixes in the syntax, contract and skill ([e915214](https://github.com/Yornik/adf-converter/commit/e915214a575f2d83ff0a523cc9dbfa3791d1da5c))
* explain Confluence value pointers and options of the other direction ([c1e7976](https://github.com/Yornik/adf-converter/commit/c1e797649bbea86774d20bf806fcce8b5c6e3586))
* **maintainers:** reflow the release re-run paragraph ([2dd27af](https://github.com/Yornik/adf-converter/commit/2dd27aff872babe245b16d06f226c974b0aa5cfa))
* mark breaking changes with ! in the PR title ([dacfe76](https://github.com/Yornik/adf-converter/commit/dacfe76de5d29a3272e5db38af85852da735492d))
* mark W_NODE_NOT_IN_TARGET as reserved ([94dfd33](https://github.com/Yornik/adf-converter/commit/94dfd337674ad75ecf20fa32532828e39c962e12))
* **md:** avoid intra-doc links to private modules ([cf04ff1](https://github.com/Yornik/adf-converter/commit/cf04ff1689e37a18ae328d7085513eda3234359b))
* note the per-code warning cap in the README validate example ([8bb212d](https://github.com/Yornik/adf-converter/commit/8bb212d8fb981665c6592be9a3dace394be0b137))
* rewrite README with install, usage, agent guidance and disclaimer ([c1aec35](https://github.com/Yornik/adf-converter/commit/c1aec3528a3ed6a78d81418c801775d993bc8a77))
* say precisely what scripts/check.sh covers ([952c205](https://github.com/Yornik/adf-converter/commit/952c205775c80cc9908272733d185d1f420c5aad))
* **scripts:** say which CI checks scripts/check.sh leaves out ([6149cb0](https://github.com/Yornik/adf-converter/commit/6149cb0a24c432b7fd727cb9eeeb921253303fde))
* **skill:** post edited content back with --images external and fix recipe pitfalls ([3d451a3](https://github.com/Yornik/adf-converter/commit/3d451a3dc4b8374c177c240420814705d3662542))
* **syntax:** match container, code block and W_ATTR_DROPPED rules to the converter ([fa1cdee](https://github.com/Yornik/adf-converter/commit/fa1cdee879d5a805e23b5bf4cfe4870821dbefa3))
* tighten README and skill wording after review ([db3004e](https://github.com/Yornik/adf-converter/commit/db3004e976c944281bcfde915cd295e46fa53d27))
* use the agentskills command of the skills-ref package ([fcadea7](https://github.com/Yornik/adf-converter/commit/fcadea7c08fac55a63be24bb7588e5c8b086a318))
