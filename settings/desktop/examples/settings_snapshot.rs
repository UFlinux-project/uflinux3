//! Generate a valid empty Settings snapshot for private D-Bus fixtures.
fn main() {
    println!(
        "{}",
        serde_json::to_string(&settings_core::Snapshot::default()).unwrap()
    );
}
