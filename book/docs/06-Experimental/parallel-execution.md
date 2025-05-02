# Parallel block execution

There is an example on how RSP can be used to execute blocks in parallel with the `continuous` binary in the `bin/continuous` folder.

The block execution statistics are stored in a Prosgres database, and the number of blocks executed in parallel can be customized with the `MAX_CONCURRENT_EXECUTIONS` environment variable.