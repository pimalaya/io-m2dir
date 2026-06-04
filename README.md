# I/O m2dir [![Documentation](https://img.shields.io/docsrs/io-m2dir?style=flat&logo=docs.rs&logoColor=white)](https://docs.rs/io-m2dir/latest/io_m2dir) [![Matrix](https://img.shields.io/badge/chat-%23pimalaya-blue?style=flat&logo=matrix&logoColor=white)](https://matrix.to/#/#pimalaya:matrix.org) [![Mastodon](https://img.shields.io/badge/news-%40pimalaya-blue?style=flat&logo=mastodon&logoColor=white)](https://fosstodon.org/@pimalaya)

m2dir client library, written in Rust.

This library is composed of 2 feature-gated layers:

- Low-level **I/O-free** coroutines: these `no_std`-compatible state machines contain the whole m2dir logic and can be used anywhere
- Mid-level **std client**: a standard, blocking m2dir client built on `std::fs`

## Table of contents

- [Features](#features)
- [Specification coverage](#specification-coverage)
- [Usage](#usage)
  - [I/O-free coroutines](#io-free-coroutines)
  - [Std client](#std-client)
- [Examples](#examples)
- [AI disclosure](#ai-disclosure)
- [License](#license)
- [Social](#social)
- [Sponsoring](#sponsoring)

## Features

- **I/O-free** coroutines: `no_std` state machines; no filesystem calls, no async runtime, no `std` required, drive against any blocking, async, or fuzz harness.
- Standard, blocking client (requires `client` feature) backed by `std::fs`.
- **m2dir delivery protocol**: the message-store coroutine writes to a temporary file in the same directory first, then atomically renames it to `<date>,<checksum>.<nonce>`.
- **Sidecar flag metadata** via `.meta/<id>.flags`: add / remove / replace flags as a separate file alongside each entry.
- **No external** base64 / rand / percent dependencies: the m2dir spec's custom base64, FNV hashing, percent-encoding, and pseudo-random byte generation are kept as in-crate `no_std` modules.

> [!TIP]
> I/O m2dir is written in [Rust](https://www.rust-lang.org/) and uses [cargo features](https://doc.rust-lang.org/cargo/reference/features.html) to gate backend support. The default feature set is declared in [Cargo.toml](./Cargo.toml) or on [docs.rs](https://docs.rs/crate/io-m2dir/latest/features).

## Specification coverage

This library implements the [m2dir](https://man.sr.ht/~bitfehler/m2dir/) mail storage format as I/O-agnostic coroutines.

| Coroutine             | What it does                                                                                  |
|-----------------------|-----------------------------------------------------------------------------------------------|
| `M2dirCreate`         | Creates an m2dir folder, its `.meta` sub-directory and the `.m2dir` marker file               |
| `M2dirDelete`         | Recursively removes an m2dir                                                                  |
| `M2dirList`           | Walks the m2store tree and surfaces every directory carrying a `.m2dir` marker                |
| `M2dirEntryStore`     | Writes a temporary file, then atomically renames to `<date>,<checksum>.<nonce>`               |
| `M2dirEntryGet`       | Locates an entry by id, reads its contents and validates the checksum                         |
| `M2dirEntryList`      | Lists every confirmed entry inside an m2dir                                                   |
| `M2dirEntryDelete`    | Removes an entry file and every sibling `.meta/<id>*` file                                    |
| `M2dirFlagAdd`        | Merges flags into an entry's `.meta/<id>.flags` sidecar                                       |
| `M2dirFlagRemove`     | Removes flags from an entry's `.meta/<id>.flags` sidecar (deletes the file when empty)        |
| `M2dirFlagSet`        | Replaces an entry's `.meta/<id>.flags` sidecar (deletes the file when the new set is empty)   |

## Usage

I/O m2dir can be consumed two ways, depending on how much of the I/O stack you want to own. Each mode is gated by cargo features.

Whichever mode you pick, every coroutine implements the `M2dirCoroutine` trait. Its `resume(arg: Option<M2dirArg>)` method returns a `M2dirCoroutineState<Yield, Return>` with two variants:

- `Yielded(Y)`: intermediate. `Y` is the per-coroutine yield type; every io-m2dir coroutine picks the standard `M2dirYield`, whose variants are `WantsPid`, `WantsRandom { len }`, `WantsDirCreate`, `WantsDirRead`, `WantsDirRemove`, `WantsFileCreate`, `WantsFileRead`, `WantsFileExists`, `WantsFileRemove`, `WantsRename`. The driver services the request and feeds back the matching `M2dirArg` variant on the next `resume`.
- `Complete(R)`: terminal. By convention `R = Result<Output, Error>` carrying the operation's final value.

### I/O-free coroutines

No features required: works in `#![no_std]`, no filesystem calls, no async runtime. You own the loop and the syscalls; the library only computes the operations to perform and consumes their results.

Drive a multi-step command (store an entry) against a blocking caller (the same shape works under async or in-memory replay):

```rust,ignore
use std::{collections::hash_map::RandomState, fs, hash::{BuildHasher, Hasher}, process};

use io_m2dir::{
    coroutine::*,
    entry::store::*,
    m2dir::types::M2dir,
    path::M2dirPath,
};

let m2dir = M2dir::from_path(M2dirPath::new("/path/to/m2dir/inbox"));
let bytes = b"From: alice@example.com\r\nSubject: Hello\r\n\r\nHello!\r\n".to_vec();

let mut coroutine = M2dirEntryStore::new(m2dir, bytes, M2dirEntryStoreOptions::default());
let mut arg: Option<M2dirArg> = None;

let entry = loop {
    match coroutine.resume(arg.take()) {
        M2dirCoroutineState::Complete(Ok(entry)) => break entry,
        M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
        M2dirCoroutineState::Yielded(M2dirYield::WantsPid) => {
            arg = Some(M2dirArg::Pid(process::id()));
        }
        M2dirCoroutineState::Yielded(M2dirYield::WantsRandom { len }) => {
            // Replace with a stronger RNG if needed; the std client
            // ships a xorshift64* helper seeded from RandomState.
            let mut out = vec![0u8; len];
            let mut state = RandomState::new().build_hasher().finish();
            for byte in &mut out {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            arg = Some(M2dirArg::Random(out));
        }
        M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => {
            for (path, bytes) in files {
                fs::write(path.as_str(), &bytes).unwrap();
            }
            arg = Some(M2dirArg::FileCreate);
        }
        M2dirCoroutineState::Yielded(M2dirYield::WantsRename(pairs)) => {
            for (from, to) in pairs {
                fs::rename(from.as_str(), to.as_str()).unwrap();
            }
            arg = Some(M2dirArg::Rename);
        }
        M2dirCoroutineState::Yielded(other) => unreachable!("M2dirEntryStore yielded {other:?}"),
    }
};

println!("stored {} at {}", entry.id(), entry.path());
```

### Std client

Enable the `client` feature (on by default). `M2dirClient::new(root)` wraps a filesystem root and exposes one method per coroutine; the resume loop is driven for you via `M2dirClient::run` and `std::fs`.

```toml,ignore
[dependencies]
io-m2dir = "0.0.1" # client is enabled by default
```

```rust,ignore
use io_m2dir::{client::M2dirClient, flag::types::M2dirFlags};

let client = M2dirClient::new("/path/to/store");

client.init_store()?;
let inbox = client.create_m2dir("inbox")?;

let bytes = b"From: alice@example.com\r\nSubject: Hello\r\n\r\nHello!\r\n".to_vec();
let entry = client.store(inbox.clone(), bytes)?;

let mut flags = M2dirFlags::default();
flags.insert("$seen");
client.add_flags(&inbox, entry.id(), flags)?;

println!("stored {} at {}", entry.id(), entry.path());
```

## Examples

Have a look at real-world projects built on top of this library:

- [Himalaya CLI](https://github.com/pimalaya/himalaya): CLI to manage emails
- [Himalaya TUI](https://github.com/pimalaya/himalaya-tui): TUI to manage emails
- [Neverest](https://github.com/pimalaya/neverest): CLI to synchronize emails

## AI disclosure

This project is developed with AI assistance. This section documents how, so users and downstream packagers can make informed decisions.

- **Tools**: Claude Code (Anthropic), Opus 4.7, invoked locally with a persistent project-scoped memory and a small set of repo-specific rules.

- **Used for**: Refactors, mechanical multi-file edits, boilerplate (feature gates, error enums, derive macros, trait impls), test scaffolding, doc polish, exploratory design conversations.

- **Not used for**: Engineering, critical code, git manipulation (commit, merge, rebase…), real-world tests.

- **Verification**: Every AI-assisted change is read, compiled, tested, and formatted before commit (`nix develop --command cargo check / cargo test / cargo
fmt`). Behavioural correctness is verified against the relevant spec, not assumed from the model output. Tests are never adjusted to fit
AI-generated code; the code is adjusted to fit correct behaviour.

- **Limitations**: AI models occasionally produce code that compiles and passes tests but is subtly wrong: off-by-one errors, missed edge cases, plausible
but nonexistent APIs, stale spec references. The verification workflow catches most of this; it does not catch all of it. Bug reports are welcome and taken
seriously.

- **Last reviewed**: 30/05/2026

## License

This project is licensed under either of:

- [MIT license](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.

## Social

- Chat on [Matrix](https://matrix.to/#/#pimalaya:matrix.org)
- News on [Mastodon](https://fosstodon.org/@pimalaya) or [RSS](https://fosstodon.org/@pimalaya.rss)
- Mail at [pimalaya.org@posteo.net](mailto:pimalaya.org@posteo.net)

## Sponsoring

[![nlnet](https://nlnet.nl/logo/banner-160x60.png)](https://nlnet.nl/)

Special thanks to the [NLnet foundation](https://nlnet.nl/) and the [European Commission](https://www.ngi.eu/) that have been financially supporting the project for years:

- 2022 → 2023: [NGI Assure](https://nlnet.nl/project/Himalaya/)
- 2023 → 2024: [NGI Zero Entrust](https://nlnet.nl/project/Pimalaya/)
- 2024 → 2026: [NGI Zero Core](https://nlnet.nl/project/Pimalaya-PIM/)
- *2027 in preparation…*

If you appreciate the project, feel free to donate using one of the following providers:

[![GitHub](https://img.shields.io/badge/-GitHub%20Sponsors-fafbfc?logo=GitHub%20Sponsors)](https://github.com/sponsors/soywod)
[![Ko-fi](https://img.shields.io/badge/-Ko--fi-ff5e5a?logo=Ko-fi&logoColor=ffffff)](https://ko-fi.com/soywod)
[![Buy Me a Coffee](https://img.shields.io/badge/-Buy%20Me%20a%20Coffee-ffdd00?logo=Buy%20Me%20A%20Coffee&logoColor=000000)](https://www.buymeacoffee.com/soywod)
[![Liberapay](https://img.shields.io/badge/-Liberapay-f6c915?logo=Liberapay&logoColor=222222)](https://liberapay.com/soywod)
[![thanks.dev](https://img.shields.io/badge/-thanks.dev-000000?logo=data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMjQuMDk3IiBoZWlnaHQ9IjE3LjU5NyIgY2xhc3M9InctMzYgbWwtMiBsZzpteC0wIHByaW50Om14LTAgcHJpbnQ6aW52ZXJ0IiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPjxwYXRoIGQ9Ik05Ljc4MyAxNy41OTdINy4zOThjLTEuMTY4IDAtMi4wOTItLjI5Ny0yLjc3My0uODktLjY4LS41OTMtMS4wMi0xLjQ2Mi0xLjAyLTIuNjA2di0xLjM0NmMwLTEuMDE4LS4yMjctMS43NS0uNjc4LTIuMTk1LS40NTItLjQ0Ni0xLjIzMi0uNjY5LTIuMzQtLjY2OUgwVjcuNzA1aC41ODdjMS4xMDggMCAxLjg4OC0uMjIyIDIuMzQtLjY2OC40NTEtLjQ0Ni42NzctMS4xNzcuNjc3LTIuMTk1VjMuNDk2YzAtMS4xNDQuMzQtMi4wMTMgMS4wMjEtMi42MDZDNS4zMDUuMjk3IDYuMjMgMCA3LjM5OCAwaDIuMzg1djEuOTg3aC0uOTg1Yy0uMzYxIDAtLjY4OC4wMjctLjk4LjA4MmExLjcxOSAxLjcxOSAwIDAgMC0uNzM2LjMwN2MtLjIwNS4xNTYtLjM1OC4zODQtLjQ2LjY4Mi0uMTAzLjI5OC0uMTU0LjY4Mi0uMTU0IDEuMTUxVjUuMjNjMCAuODY3LS4yNDkgMS41ODYtLjc0NSAyLjE1NS0uNDk3LjU2OS0xLjE1OCAxLjAwNC0xLjk4MyAxLjMwNXYuMjE3Yy44MjUuMyAxLjQ4Ni43MzYgMS45ODMgMS4zMDUuNDk2LjU3Ljc0NSAxLjI4Ny43NDUgMi4xNTR2MS4wMjFjMCAuNDcuMDUxLjg1NC4xNTMgMS4xNTIuMTAzLjI5OC4yNTYuNTI1LjQ2MS42ODIuMTkzLjE1Ny40MzcuMjYuNzMyLjMxMi4yOTUuMDUuNjIzLjA3Ni45ODQuMDc2aC45ODVabTE0LjMxNC03LjcwNmgtLjU4OGMtMS4xMDggMC0xLjg4OC4yMjMtMi4zNC42NjktLjQ1LjQ0Ni0uNjc3IDEuMTc3LS42NzcgMi4xOTVWMTQuMWMwIDEuMTQ0LS4zNCAyLjAxMy0xLjAyIDIuNjA2LS42OC41OTMtMS42MDUuODktMi43NzQuODloLTIuMzg0di0xLjk4OGguOTg0Yy4zNjIgMCAuNjg4LS4wMjcuOTgtLjA4LjI5Mi0uMDU1LjUzOC0uMTU3LjczNy0uMzA4LjIwNC0uMTU3LjM1OC0uMzg0LjQ2LS42ODIuMTAzLS4yOTguMTU0LS42ODIuMTU0LTEuMTUydi0xLjAyYzAtLjg2OC4yNDgtMS41ODYuNzQ1LTIuMTU1LjQ5Ny0uNTcgMS4xNTgtMS4wMDQgMS45ODMtMS4zMDV2LS4yMTdjLS44MjUtLjMwMS0xLjQ4Ni0uNzM2LTEuOTgzLTEuMzA1LS40OTctLjU3LS43NDUtMS4yODgtLjc0NS0yLjE1NXYtMS4wMmMwLS40Ny0uMDUxLS44NTQtLjE1NC0xLjE1Mi0uMTAyLS4yOTgtLjI1Ni0uNTI2LS40Ni0uNjgyYTEuNzE5IDEuNzE5IDAgMCAwLS43MzctLjMwNyA1LjM5NSA1LjM5NSAwIDAgMC0uOTgtLjA4MmgtLjk4NFYwaDIuMzg0YzEuMTY5IDAgMi4wOTMuMjk3IDIuNzc0Ljg5LjY4LjU5MyAxLjAyIDEuNDYyIDEuMDIgMi42MDZ2MS4zNDZjMCAxLjAxOC4yMjYgMS43NS42NzggMi4xOTUuNDUxLjQ0NiAxLjIzMS42NjggMi4zNC42NjhoLjU4N3oiIGZpbGw9IiNmZmYiLz48L3N2Zz4=)](https://thanks.dev/soywod)
[![PayPal](https://img.shields.io/badge/-PayPal-0079c1?logo=PayPal&logoColor=ffffff)](https://www.paypal.com/paypalme/soywod)
