//! Exercises every branch of the worm ABI from the guest side

larvae_worm::frontend!(|source: &str, config: &str| -> Result<String, String> {
    match source {
        // a worm reporting a problem rather than producing output
        "FAIL" => Err(format!("refused, config was {config:?}")),

        // a worm hitting a bug, which reaches the host as a trap
        "TRAP" => panic!("worm exploded"),

        _ => Ok(format!("{source}|{config}")),
    }
});
