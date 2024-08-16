use sp1_helper::{build_program_with_args, BuildArgs};

fn main() {
    build_program_with_args(&format!("../{}", "client-eth"), BuildArgs { ..Default::default() });
    build_program_with_args(&format!("../{}", "client-op"), BuildArgs { ..Default::default() });
}
