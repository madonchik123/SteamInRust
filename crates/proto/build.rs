//! Compiles the vendored Steam .proto files with a pure-Rust compiler (protox),
//! so no `protoc` install is needed. To support a new message, drop its .proto
//! file into ./protos (from github.com/SteamTracking/Protobufs, `steam/`) or
//! ./protos/webui (from `webui/`) and add it below.

/// Client/CM protocol (flat module at the crate root).
const FILES: &[&str] = &[
    "steammessages_base.proto",
    "enums_clientserver.proto",
    "steammessages_clientserver_login.proto",
    "steammessages_clientserver.proto",
    "steammessages_clientserver_2.proto",
    "steammessages_clientserver_friends.proto",
    "steammessages_clientserver_appinfo.proto",
    "steammessages_auth.steamclient.proto",
    "steammessages_friendmessages.steamclient.proto",
    "steammessages_player.steamclient.proto",
    "content_manifest.proto",
    "steammessages_store.steamclient.proto",
    "steammessages_publishedfile.steamclient.proto",
    "steammessages_contentsystem.steamclient.proto",
    "steammessages_econ.steamclient.proto",
    "steammessages_cloud.steamclient.proto",
    "steammessages_inventory.steamclient.proto",
];

/// Web UI services (cart, wishlist, store query). Separate module because
/// they redefine some client types.
const WEBUI: &[&str] = &["service_accountcart.proto", "service_wishlist.proto", "service_storequery.proto", "service_steamnotification.proto", "service_cloudconfigstore.proto"];

fn main() {
    println!("cargo:rerun-if-changed=protos");
    let fds = protox::compile(FILES, ["protos"]).expect("compile protos");
    prost_build::Config::new().compile_fds(fds).expect("generate rust from protos");

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("webui");
    std::fs::create_dir_all(&out).unwrap();
    let fds = protox::compile(WEBUI, ["protos/webui"]).expect("compile webui protos");
    prost_build::Config::new().out_dir(&out).compile_fds(fds).expect("generate webui protos");
}
