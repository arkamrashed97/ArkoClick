//! Opens the target picker with the Arabic hint, for visual checks. Esc closes it.
fn main() {
    let hint = "انقر على المكان الذي تريد أن يضغط عليه ArkoClick\u{200F}      ·      \u{200F}Esc للإلغاء";
    let p = arkoclick::win::picker::Picker::open(hint.to_string(), true);
    loop {
        if let Some(outcome) = p.poll() {
            println!("{outcome:?}");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
