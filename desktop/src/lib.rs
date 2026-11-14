#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("Psychotalk desktop requires macOS or Linux");
pub mod envelopes;

mod authorization;
pub mod engine;
pub mod server;
