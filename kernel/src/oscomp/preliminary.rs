pub fn run() -> bool {
    crate::println!("sudoos-diag: entering preliminary oscomp runner");
    crate::user::verify_sdcard_sample();
    crate::user::verify_sdcard_all_scripts()
}
