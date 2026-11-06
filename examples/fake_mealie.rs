//! Runs the tests' fake Mealie (seeded from `tests/fixtures/mealie/`) on a port, to try the
//! backend against it by hand:
//!
//! ```bash
//! cargo run --example fake_mealie -- 127.0.0.1:9925
//! MEALIE_URL=http://127.0.0.1:9925 MEALIE_TOKEN=fake-mealie-token HQ_SETTINGS=<settings with
//!   shopping.mealie.list = "Groceries"> cargo run
//! curl -X POST http://127.0.0.1:9925/__fake/down   # and /__fake/up
//! ```

#[path = "../tests/common/fake_mealie.rs"]
mod fake_mealie;

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9925".into())
        .parse()
        .expect("an address such as 127.0.0.1:9925");
    let fake = fake_mealie::FakeMealie::from_fixtures_on(Some(addr)).await;
    println!("fake Mealie on {} (token {})", fake.url, fake_mealie::TOKEN);
    tokio::signal::ctrl_c().await.unwrap();
}
