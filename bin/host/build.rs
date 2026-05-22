use sp1_build::{build_program, build_program_with_args, BuildArgs};

fn main() {
    // When the host is built with `--features arena`, build the rsp-client guest with the arena
    // feature too, so host and guest agree on the (arena-encoded) ClientExecutorInput wire type.
    if std::env::var("CARGO_FEATURE_ARENA").is_ok() {
        build_program_with_args(
            "../client",
            BuildArgs { features: vec!["arena".to_string()], ..Default::default() },
        );
    } else {
        build_program("../client");
    }
    build_program("../client-op");
}
