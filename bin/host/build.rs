use sp1_helper::{build_program_with_args, BuildArgs};

fn main() {
    build_program_with_args(
        &format!("../{}", "client-eth"),
        BuildArgs { ignore_rust_version: true, ..Default::default() },
    );
    build_program_with_args(
        &format!("../{}", "client-op"),
        BuildArgs { ignore_rust_version: true, ..Default::default() },
    );
    build_program_with_args(
        &format!("../{}", "client-linea"),
        BuildArgs { ignore_rust_version: true, ..Default::default() },
    );
}
