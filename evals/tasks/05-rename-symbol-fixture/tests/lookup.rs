use config_lookup::{port, Config};

#[test]
fn reads_values() {
    let c = Config::parse("name = mira\nport = 9000");
    assert_eq!(c.lookup("name"), Some("mira"));
    assert_eq!(c.lookup("missing"), None);
    assert_eq!(c.get_or("missing", "x"), "x");
    assert_eq!(port(&c), 9000);
}
