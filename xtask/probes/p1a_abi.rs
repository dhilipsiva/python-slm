unsafe extern "C" {
    fn p1a_c_probe(left: u64, right: u64) -> u64;
    fn p1a_cpp_probe(left: u64, right: u64) -> u64;
}

fn main() {
    // SAFETY: both symbols use the frozen C ABI, accept two u64 values, return u64,
    // and are linked from the two source files qualified by this probe.
    let (c_result, cpp_result) = unsafe { (p1a_c_probe(40, 2), p1a_cpp_probe(40, 2)) };
    assert_eq!(c_result, 3_137);
    assert_eq!(cpp_result, 150);
    println!("P1A_ABI_PASS c={c_result} cpp={cpp_result}");
}
