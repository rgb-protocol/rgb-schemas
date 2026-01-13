# RGB Schemas

[![Build](https://github.com/rgb-protocol/rgb-schemas/actions/workflows/build.yml/badge.svg)](https://github.com/rgb-protocol/rgb-schemas/actions/workflows/build.yml)
[![Tests](https://github.com/rgb-protocol/rgb-schemas/actions/workflows/test.yml/badge.svg)](https://github.com/rgb-protocol/rgb-schemas/actions/workflows/test.yml)
[![Lints](https://github.com/rgb-protocol/rgb-schemas/actions/workflows/lint.yml/badge.svg)](https://github.com/rgb-protocol/rgb-schemas/actions/workflows/lint.yml)
[![codecov](https://codecov.io/gh/rgb-protocol/rgb-schemas/branch/master/graph/badge.svg)](https://app.codecov.io/gh/rgb-protocol/rgb-schemas)

[![crates.io](https://img.shields.io/crates/v/rgb-schemas)](https://crates.io/crates/rgb-schemas)
[![Docs](https://docs.rs/rgb-schemas/badge.svg)](https://docs.rs/rgb-schemas)
[![Apache-2 licensed](https://img.shields.io/crates/l/rgb-schemas)](./LICENSE)

This repository provides rust source code and compiled versions of RGB
contract schemata recommended for the use by contract developers.

RGB is confidential & scalable client-validated smart contracts for Bitcoin &
Lightning. To learn more about RGB please check [RGB website][Site].

## Catalog

This repository provides the following RGB schemata:

* __Non-inflatable assets (NIA)__.
  This is the simplest form of a fungible asset/token, which doesn't provide
  such features as secondary issue, ability to change asset name and
  parameters, ability to burn the asset.

* __Unique digital asset (UDA)__.
  This is the simplest form of an NFT, which has one issuance of a single
  non-fungible and non-fractionable token with a representative attached
  media file and a preview.

* __Collectible fungible assets (CFA)__.
  This is the simplest form of collectible fungible assets.

* __Permissioned fungible assets (PFA)__.
  **Not production-ready**
  This is a fungible asset in which the issuer needs to approve every transfer.

* __Inflatable fungible assets (IFA)__.
  **Not production-ready**
  This is a fungible asset that supports *inflate*, *burn* and *link* transitions.

## License

See [LICENSE](LICENSE) file.


[Site]: https://rgb.info
