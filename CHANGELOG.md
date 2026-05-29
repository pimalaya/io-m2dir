# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Replaced the per-coroutine `M2dir*Arg` enums and the single shared `M2dirCoroutineState<T, E>` (`Done` / `Wants*` / `Err`) with the generator-shape `M2dirCoroutineState<Y, R>` (`Yielded` / `Complete`). Every coroutine now implements `M2dirCoroutine` with `type Yield = M2dirYield` and `type Return = Result<Output, Error>`; the previously per-coroutine reply enums collapse into a single crate-wide `M2dirArg` variant set matching `M2dirYield`. `M2dirMessageGetOk` was renamed to `M2dirMessageGetOutput` to align with the per-coroutine output struct convention. `M2dirClient::run<C, T, E>` is now generic over any standard-Yield coroutine and services every `M2dirYield` variant via `std::fs`; each client method becomes a one-line `self.run(coroutine)` call.
