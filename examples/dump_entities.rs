fn main() {
    let text = std::fs::read_to_string("/tmp/herdr-extract-demo.txt").unwrap_or_else(|_| {
        r#"
contact jon@arcs.health
api.staging.example.internal:8443
550e8400-e29b-41d4-a716-446655440000
wiseman-8ure PROJ-1234 #42
DATABASE_URL
https://docs.example.com/guide
Jon Roosevelt Covenant Clinics
Board of Directors
cargo test --release
0123456789abcdef v1.2.3
error: failed to compile
/srv/releases/staging/app.tar.gz
10.0.0.12
"#
        .into()
    });
    let items = herdr_extractor::extract::extract_items_from_visible_text(&text);
    for item in &items {
        println!("{:<8} {}", item.kind.stable_key(), item.text);
    }
    println!("--- total {} ---", items.len());
}
