#![deny(unsafe_op_in_unsafe_fn)]

mod cli;
mod error;
mod hash;
mod p0;
mod p0a;
mod process;
mod publication;
mod time;

use std::ffi::OsString;

pub fn entry(args: impl IntoIterator<Item = OsString>) -> i32 {
    match cli::run(args) {
        Ok(value) => {
            match serde_json::to_string(&value) {
                Ok(line) => println!("{line}"),
                Err(error) => {
                    eprintln!(
                        "{{\"schema\":\"python-slm-xtask-error-v1\",\"code\":\"JSON_SERIALIZATION_FAILED\",\"category\":\"internal\",\"message\":{}}}",
                        serde_json::to_string(&error.to_string())
                            .unwrap_or_else(|_| "\"serialization failed\"".to_owned())
                    );
                    return 1;
                }
            }
            0
        }
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::to_string(&error).unwrap_or_else(|_| {
                    "{\"schema\":\"python-slm-xtask-error-v1\",\"code\":\"ERROR_SERIALIZATION_FAILED\",\"category\":\"internal\",\"message\":\"error serialization failed\",\"remediation\":\"Inspect the xtask implementation.\"}".to_owned()
                })
            );
            error.exit_code()
        }
    }
}
