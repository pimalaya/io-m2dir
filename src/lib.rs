#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![doc = include_str!("../README.md")]

#[macro_use]
extern crate alloc;
#[cfg(feature = "client")]
extern crate std;

pub mod base64;
#[cfg(feature = "client")]
pub mod client;
pub mod coroutine;
pub mod coroutines;
pub mod entry;
pub mod flag;
pub mod fnv;
pub mod m2dir;
pub mod m2store;
pub mod parse;
pub mod path;
pub mod percent;
