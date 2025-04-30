# CSV Report

Statistics on block execution can be saved to a CSV file with the `--report-path <path-to-a-csv-file>` argument.

To add precompile tracking to the CSV file, use the `--precompile-tracking` argument. Similarly, use `--opcode-tracking` to include opcode tracking.

:::warning

Using `--opcode-tracking` argument results in substantial performance degradation and significantly increased cycle counts.

:::