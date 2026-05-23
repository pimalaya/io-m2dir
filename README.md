# io-m2dir

Sans-I/O coroutines for the [m2dir] mail storage format, with optional blocking std client.

The crate exposes resumable, I/O-free state machines that describe the operations needed to manipulate an m2store: listing m2dirs, creating folders, storing and reading entries, and round-tripping the `.meta/<id>.flags` sidecar. A blocking [`std::fs`] driver lives behind the `client` feature.

The no_std core preserves the original [m2dir] specification's custom base64, FNV hashing, percent-encoding, and pseudo-random byte generation as in-crate modules: no external base64/rand/percent dependencies are pulled in.

[m2dir]: https://man.sr.ht/~bitfehler/m2dir/
