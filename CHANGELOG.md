# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Replaced the per-coroutine `M2dir*Arg` enums and the single shared `M2dirCoroutineState<T, E>` (`Done` / `Wants*` / `Err`) with the generator-shape `M2dirCoroutineState<Y, R>` (`Yielded` / `Complete`). Every coroutine now implements `M2dirCoroutine` with `type Yield = M2dirYield` and `type Return = Result<Output, Error>`; the previously per-coroutine reply enums collapse into a single crate-wide `M2dirArg` variant set matching `M2dirYield`. `M2dirMessageGetOk` was renamed to `M2dirMessageGetOutput` to align with the per-coroutine output struct convention. `M2dirClient::run<C, T, E>` is now generic over any standard-Yield coroutine and services every `M2dirYield` variant via `std::fs`; each client method becomes a one-line `self.run(coroutine)` call.

- Renamed the protocol-agnostic modules and types away from mail-specialised vocabulary to align io-m2dir with io-maildir:
  - Module `m2store` renamed to `store`; type `M2store` renamed to `M2dirStore`; error `LoadM2storeError` renamed to `LoadM2dirStoreError`.
  - Module `mailbox` folded into `m2dir/`; `m2dir.rs` moved to `m2dir/types.rs`; coroutines `M2dirMailboxCreate`, `M2dirMailboxDelete`, `M2dirMailboxList` (and their `*Error`/`*Options` siblings) renamed to `M2dirCreate`, `M2dirDelete`, `M2dirList`.
  - Module `message` folded into `entry/`; `entry.rs` moved to `entry/types.rs`; coroutines `M2dirMessageDelete`, `M2dirMessageGet`, `M2dirMessageList`, `M2dirMessageStore` (and their `*Error`/`*Options`/`*Output` siblings) renamed to `M2dirEntryDelete`, `M2dirEntryGet`, `M2dirEntryList`, `M2dirEntryStore`.
  - `M2dirClient` methods `create_mailbox`, `delete_mailbox`, `list_mailboxes`, `delete_message` renamed to `create_m2dir`, `delete_m2dir`, `list_m2dirs`, `delete_entry`.

- Replaced the four crate-local utility modules with established external crates and an inline helper, dropping ~250 lines of vendored encoders/parsers:
  - `base64` module removed; uses the `base64` crate (URL-safe alphabet with padding, no_std + `alloc`).
  - `percent` module removed; uses the `percent-encoding` crate with an `M2DIR_PCT` `AsciiSet` over `%`, `/`, `\\`, and all control bytes.
  - `fnv` module removed; the 9-line FNV-1a-64 hash is inlined into its sole call site (`entry/types.rs::write_checksum`).
  - `parse` module pruned: the public `Datetime` struct, `Rfc2822Error`, and the crate-root `pub mod parse` are gone. The `Date:` header extraction now goes through `mail-parser` (`MessageParser::new().with_date_headers()`) when the `client` feature is on (mail-parser is `std`-only). The original hand-rolled RFC 2822 parser is kept as a private `cfg(not(feature = "client"))` fallback so no_std builds without `client` retain real date prefixes (rather than degrading to epoch). Both branches return `Option<String>` formatted as `YYYY-MM-DDTHH:MM:SS[Z|±HHMM]`; the epoch is used only when the message has no parseable `Date:` header in either path.
