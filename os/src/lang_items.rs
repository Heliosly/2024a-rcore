//! The panic handler


use core::panic::PanicInfo;

use crate::sbi::shutdown;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {

   
    // use crate::sbi::shutdown;
    if let Some(location) = _info.location() {
        println!(
            "[kernel] Panicked at {}:{} {}",
            location.file(),
            location.line(),
            _info.message()
        );
    } else {
        println!("[kernel] Panicked: {}", _info.message());
    }
    shutdown()
}
