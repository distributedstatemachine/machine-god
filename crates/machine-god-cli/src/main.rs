fn main() {
    println!(
        "machine-god {} (engine API {})",
        env!("CARGO_PKG_VERSION"),
        machine_god_native::supported_core_api_version()
    );
}
