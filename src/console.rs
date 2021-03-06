#[cfg(feature = "desktop")]
#[path = "./console/desktop_console.rs"]
pub mod console_util;

#[cfg(feature = "web")]
#[path = "./console/web_console.rs"]
pub mod console_util;

///Platform specific implementations are here.
pub use console_util::*;


#[macro_export]
macro_rules! console_log {
    // Note that this is using the `log` function imported above during
    // `bare_bones`
    ($($t:tt)*) => (console_write(&format_args!($($t)*).to_string()))
}