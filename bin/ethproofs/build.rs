use sp1_build::{build_program_with_args, BuildArgs};

fn main() {
    // Build the `rsp-client` guest ELF. When this host binary is built with its own `arena`
    // feature, cargo sets `CARGO_FEATURE_ARENA`; forward it to the guest so the ELF's MPT
    // backend and witness codec match the host's (`rsp-host-executor/arena`).
    let features = if std::env::var_os("CARGO_FEATURE_ARENA").is_some() {
        vec!["arena".to_string()]
    } else {
        Vec::new()
    };
    build_program_with_args("../client", BuildArgs { features, ..Default::default() });
}
