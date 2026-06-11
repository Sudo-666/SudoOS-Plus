use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    crate::println!();
    crate::println!("================ KERNEL PANIC ================");
    crate::println!("{info}");
    crate::println!("==============================================");

    loop {
        crate::arch::cpu::wait_for_interrupt();
    }
}
