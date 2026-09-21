// MANEDRS_PLAN.md §4.3, second row of the table: concurrent mnd_request calls
// on one handle are unsafe unless the script is non-routed, so `Script::run`
// takes `&mut self` and two simultaneous calls cannot be spelled.
//
// manedpy takes an uncontended mutex on every call to make this case correct.
// Rust gets the same guarantee from the borrow checker at zero runtime cost —
// and here is the proof, in the form of a program that does not build.

fn main() {
    let mut script = maned::load("plain.mnd").unwrap();
    let payload = serde_json::json!({ "x": [[1, 0], [0, 1]] });

    std::thread::scope(|scope| {
        scope.spawn(|| script.run(&payload));
        scope.spawn(|| script.run(&payload));
    });
}
