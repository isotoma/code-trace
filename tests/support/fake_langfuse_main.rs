//! Standalone fake Langfuse for the Track 1 container harness — the exact
//! implementation Track 2 tests use in-process, exposed on a fixed port.
//! Build with: cargo build --bin fake-langfuse --features harness

#[allow(dead_code)]
#[path = "fake_langfuse.rs"]
mod fake_langfuse;

fn main() {
    let addr =
        std::env::var("FAKE_LANGFUSE_ADDR").unwrap_or_else(|_| "0.0.0.0:3080".to_string());
    let fake = fake_langfuse::FakeLangfuse::start_on(&addr);
    println!("fake langfuse listening on {}", fake.url());
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
