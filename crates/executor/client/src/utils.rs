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
