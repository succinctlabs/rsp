fn main() {
    #[cfg(feature = "embedded-programs")]
    {
        use sp1_build::build_program;

        // Build the Ethereum client program
        build_program("../../bin/client");

        // Build the Optimism client program
        build_program("../../bin/client-op");
    }
}
