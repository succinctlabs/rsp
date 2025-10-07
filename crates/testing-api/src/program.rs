use crate::error::TestingApiError;

/// The type of RSP client program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramType {
    /// Ethereum mainnet client program.
    Eth,
    /// Optimism client program.
    Op,
}

/// Get the compiled ELF bytes for the specified program type.
///
/// This function requires the `embedded-programs` feature to be enabled.
/// The ELF is embedded at compile time, so it's guaranteed to match the
/// version of the crate.
///
/// # Arguments
///
/// * `program_type` - The type of program to retrieve (Eth or Op)
///
/// # Returns
///
/// A vector containing the ELF bytes, or an error if the feature is not enabled.
///
/// # Example
///
/// ```no_run
/// use rsp_testing_api::{get_program_elf, ProgramType};
///
/// let eth_elf = get_program_elf(ProgramType::Eth)?;
/// std::fs::write("eth-program.elf", eth_elf)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn get_program_elf(program_type: ProgramType) -> Result<Vec<u8>, TestingApiError> {
    #[cfg(feature = "embedded-programs")]
    {
        use sp1_sdk::include_elf;

        let elf = match program_type {
            ProgramType::Eth => include_elf!("rsp-client"),
            ProgramType::Op => include_elf!("rsp-client-op"),
        };

        Ok(elf.to_vec())
    }

    #[cfg(not(feature = "embedded-programs"))]
    {
        let _ = program_type; // Suppress unused variable warning
        Err(TestingApiError::FeatureNotEnabled(
            "embedded-programs - add this feature to Cargo.toml to embed program ELFs",
        ))
    }
}

#[cfg(test)]
#[cfg(feature = "embedded-programs")]
mod tests {
    use super::*;

    #[test]
    fn test_get_eth_program() {
        let elf = get_program_elf(ProgramType::Eth).unwrap();
        assert!(!elf.is_empty(), "ETH ELF should not be empty");
    }

    #[test]
    fn test_get_op_program() {
        let elf = get_program_elf(ProgramType::Op).unwrap();
        assert!(!elf.is_empty(), "OP ELF should not be empty");
    }
}
