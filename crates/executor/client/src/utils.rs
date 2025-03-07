/// Profile the given code block cycle count.
#[allow(unused_macros)]
macro_rules! profile {
    ($name:expr, $block:block) => {{
        #[cfg(target_os = "zkvm")]
        {
            println!("cycle-tracker-start: {}", $name);
            let result = (|| $block)();
            println!("cycle-tracker-end: {}", $name);
            result
        }

        #[cfg(not(target_os = "zkvm"))]
        {
            $block
        }
    }};
}

/// Profile the given code block and add the cycle count to the execution report.
macro_rules! profile_report {
    ($name:expr, $block:block) => {{
        #[cfg(target_os = "zkvm")]
        {
            println!("cycle-tracker-report-start: {}", $name);
            let result = (|| $block)();
            println!("cycle-tracker-report-end: {}", $name);
            result
        }

        #[cfg(not(target_os = "zkvm"))]
        {
            $block
        }
    }};
}
