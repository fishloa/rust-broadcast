#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod address;
pub mod client;
pub mod error;
pub mod ingest;
pub mod limits;
pub mod mpd;
pub mod selection;
pub mod timing;
