//! Generated Steam protobuf types. The upstream files declare no package, so
//! everything lands in one flat module; web UI services live in [`webui`].

#![allow(clippy::all, non_camel_case_types, non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/_.rs"));

/// Services the Steam web UI uses: cart, wishlist, store query.
pub mod webui {
    #![allow(clippy::all, non_camel_case_types, non_snake_case, dead_code)]
    include!(concat!(env!("OUT_DIR"), "/webui/_.rs"));
}

pub use prost::Message;

/// Steam's "no job" sentinel used in `jobid_source` / `jobid_target`.
pub const NO_JOB: u64 = u64::MAX;
