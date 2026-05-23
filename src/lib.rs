#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![doc = include_str!("../README.md")]

#[macro_use]
extern crate alloc;
#[cfg(feature = "client")]
extern crate std;

pub mod base64;
pub mod fnv;
pub mod parse;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod coroutines;
#[cfg(feature = "client")]
pub mod entry;
#[cfg(feature = "client")]
pub mod flag;
#[cfg(feature = "client")]
pub mod m2dir;
#[cfg(feature = "client")]
pub mod m2store;
#[cfg(feature = "client")]
pub mod percent;
#[cfg(feature = "client")]
pub mod rand;
